//! Native driver loading (Phase 8, Layer 2): `.so` (Linux) and `.dll` (Windows)
//! drivers through `libloading`, behind the `native-loader` feature.
//!
//! # Trust model
//!
//! A native driver is **trusted code**. It runs in-process with full access to
//! the address space, so this loader does not sandbox it. Capability checks
//! happen at activation time only (through the registry policy). Execution-time
//! enforcement exists only for WASM drivers. Use the WASM sandbox for anything
//! that is not trusted.
//!
//! # ABI
//!
//! A driver module exports three `extern "C"` symbols, which
//! [`export_native_driver!`] generates:
//! - `_nova_driver_abi_version() -> u32`, which must equal [`NATIVE_ABI_VERSION`].
//! - `_nova_driver_create() -> *mut c_void`, which boxes the driver.
//! - `_nova_driver_destroy(*mut c_void)`, which frees it.
//!
//! The `dyn Driver` vtable is Rust ABI, not C ABI. The module must therefore be
//! built with the same `rustc` and the same `tpt-nova-drivers` version as the
//! host. The version number guards against an incompatible *declaration* of the
//! export shape. It does not guard against a different compiler.
//!
//! # Hot reload
//!
//! The loader copies the file to a unique temporary path before loading, so the
//! original can be rebuilt while the copy stays mapped.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::driver::{CapabilitySet, Driver, DriverContext, DriverError, DriverId, DriverVersion};
use tpt_nova_ecs::{ComponentTypeId, World};

/// Bump when the exported symbols or their signatures change.
pub const NATIVE_ABI_VERSION: u32 = 1;

/// Symbol names the host looks up.
pub mod symbols {
    pub const ABI_VERSION: &[u8] = b"_nova_driver_abi_version";
    pub const CREATE: &[u8] = b"_nova_driver_create";
    pub const DESTROY: &[u8] = b"_nova_driver_destroy";
}

/// Generate the C ABI exports for a `Default + Driver` type.
///
/// Use it once, in the `cdylib` crate that defines the driver:
/// ```ignore
/// tpt_nova_drivers::export_native_driver!(MyGravityDriver);
/// ```
#[macro_export]
macro_rules! export_native_driver {
    ($ty:ty) => {
        #[no_mangle]
        pub extern "C" fn _nova_driver_abi_version() -> u32 {
            $crate::native::NATIVE_ABI_VERSION
        }

        #[no_mangle]
        pub extern "C" fn _nova_driver_create() -> *mut ::core::ffi::c_void {
            let driver: ::std::boxed::Box<dyn $crate::Driver> =
                ::std::boxed::Box::new(<$ty as ::core::default::Default>::default());
            ::std::boxed::Box::into_raw(::std::boxed::Box::new(driver)) as *mut ::core::ffi::c_void
        }

        /// # Safety
        /// `ptr` must come from `_nova_driver_create` and be passed exactly once.
        #[no_mangle]
        pub unsafe extern "C" fn _nova_driver_destroy(ptr: *mut ::core::ffi::c_void) {
            if !ptr.is_null() {
                drop(::std::boxed::Box::from_raw(
                    ptr as *mut ::std::boxed::Box<dyn $crate::Driver>,
                ));
            }
        }
    };
}

/// Why a native driver could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum NativeLoadError {
    #[error("could not copy {path} for loading: {source}")]
    Copy {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not load library: {0}")]
    Library(String),
    #[error("module does not export `{0}`")]
    MissingSymbol(&'static str),
    #[error("ABI mismatch: host={host} module={module}")]
    AbiMismatch { host: u32, module: u32 },
    #[error("module's create function returned a null driver")]
    NullInstance,
}

type AbiFn = unsafe extern "C" fn() -> u32;
type CreateFn = unsafe extern "C" fn() -> *mut c_void;
type DestroyFn = unsafe extern "C" fn(*mut c_void);

/// A driver loaded from a dynamic library. Delegates every `Driver` call to the
/// module's own instance.
pub struct NativeDriver {
    /// `*mut Box<dyn Driver>` as created by the module. Valid until `drop`.
    instance: *mut Box<dyn Driver>,
    destroy: DestroyFn,
    /// Kept last so the code stays mapped until the instance is destroyed in `Drop`.
    _library: libloading::Library,
    /// Temporary copy that was loaded. Removed on a best-effort basis.
    copy: PathBuf,
}

// SAFETY: the module's `Driver` is `Send + Sync` (a supertrait), and the
// instance is only reached through `&self`. Destruction happens once, in `Drop`.
unsafe impl Send for NativeDriver {}
unsafe impl Sync for NativeDriver {}

static COPY_COUNTER: AtomicU64 = AtomicU64::new(0);

impl NativeDriver {
    /// Load the driver at `path`, after checking the ABI version. On any failure
    /// the temporary copy is removed.
    pub fn load(path: &Path) -> Result<Self, NativeLoadError> {
        let copy = unique_copy_path(path);
        if let Err(source) = std::fs::create_dir_all(copy.parent().unwrap_or(Path::new(".")))
            .and_then(|_| std::fs::copy(path, &copy).map(|_| ()))
        {
            return Err(NativeLoadError::Copy {
                path: path.to_path_buf(),
                source,
            });
        }
        match open_copy(&copy) {
            Ok((library, destroy, instance)) => Ok(NativeDriver {
                instance,
                destroy,
                _library: library,
                copy,
            }),
            Err(e) => {
                let _ = std::fs::remove_file(&copy);
                Err(e)
            }
        }
    }

    fn inner(&self) -> &dyn Driver {
        // SAFETY: `instance` is non-null and valid until `drop` (see `load`).
        unsafe { (*self.instance).as_ref() }
    }
}

/// Open the copied module, check its ABI, and create its driver instance.
fn open_copy(
    copy: &Path,
) -> Result<(libloading::Library, DestroyFn, *mut Box<dyn Driver>), NativeLoadError> {
    // SAFETY: loading a library runs its initialisers. The caller chose this
    // file, and native drivers are trusted (see the module docs).
    let library = unsafe { libloading::Library::new(copy) }
        .map_err(|e| NativeLoadError::Library(e.to_string()))?;

    // SAFETY: the symbols are looked up by name and copied out as function
    // pointers. They stay valid because `library` is returned alongside them.
    let (module_abi, create, destroy) = unsafe {
        let abi: libloading::Symbol<AbiFn> = library
            .get(symbols::ABI_VERSION)
            .map_err(|_| NativeLoadError::MissingSymbol("_nova_driver_abi_version"))?;
        let create: libloading::Symbol<CreateFn> = library
            .get(symbols::CREATE)
            .map_err(|_| NativeLoadError::MissingSymbol("_nova_driver_create"))?;
        let destroy: libloading::Symbol<DestroyFn> = library
            .get(symbols::DESTROY)
            .map_err(|_| NativeLoadError::MissingSymbol("_nova_driver_destroy"))?;
        (abi(), *create, *destroy)
    };
    if module_abi != NATIVE_ABI_VERSION {
        return Err(NativeLoadError::AbiMismatch {
            host: NATIVE_ABI_VERSION,
            module: module_abi,
        });
    }

    // SAFETY: `create` is the module's exported constructor with the agreed signature.
    let raw = unsafe { create() };
    if raw.is_null() {
        return Err(NativeLoadError::NullInstance);
    }
    Ok((library, destroy, raw as *mut Box<dyn Driver>))
}

fn unique_copy_path(source: &Path) -> PathBuf {
    let n = COPY_COUNTER.fetch_add(1, Ordering::Relaxed);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let name = source
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| "driver".into());
    std::env::temp_dir()
        .join("tpt-nova-drivers")
        .join(format!("{}-{stamp}-{n}-{name}", std::process::id()))
}

impl Drop for NativeDriver {
    fn drop(&mut self) {
        // SAFETY: `instance` came from this module's `create` and is destroyed
        // exactly once. The library is still loaded because fields drop after this.
        unsafe { (self.destroy)(self.instance as *mut c_void) };
        // Best effort: on Windows the copy is still mapped until the library is
        // unloaded, so removal may fail and is ignored.
        let _ = std::fs::remove_file(&self.copy);
    }
}

impl Driver for NativeDriver {
    fn id(&self) -> DriverId {
        self.inner().id()
    }
    fn version(&self) -> DriverVersion {
        self.inner().version()
    }
    fn component(&self) -> ComponentTypeId {
        self.inner().component()
    }
    fn requested_capabilities(&self) -> CapabilitySet {
        self.inner().requested_capabilities()
    }
    fn update(&self, ctx: &DriverContext, world: &mut World) -> Result<(), DriverError> {
        self.inner().update(ctx, world)
    }
    fn migrate_from(
        &self,
        from: DriverVersion,
        data: &serde_json::Value,
    ) -> Option<Result<serde_json::Value, DriverError>> {
        self.inner().migrate_from(from, data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_a_copy_error() {
        let Err(err) = NativeDriver::load(Path::new("definitely/not/here.so")) else {
            panic!("a missing file must not load");
        };
        assert!(matches!(err, NativeLoadError::Copy { .. }), "{err:?}");
    }

    /// A file that is not a loadable library must fail cleanly and leave no temp
    /// copy behind. Loading a copy of a real system library is avoided on purpose:
    /// a relocated `kernel32.dll` crashes the host.
    #[test]
    fn a_non_library_file_is_refused_and_its_copy_removed() {
        let dir = std::env::temp_dir().join("tpt-nova-drivers-test");
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("not-a-library.so");
        std::fs::write(&fake, b"this is not an ELF or PE image").unwrap();

        let Err(err) = NativeDriver::load(&fake) else {
            panic!("a non-library file must not load");
        };
        assert!(matches!(err, NativeLoadError::Library(_)), "{err:?}");

        // The only copies this process could have made are under the drivers temp dir.
        let leftovers: Vec<_> = std::fs::read_dir(std::env::temp_dir().join("tpt-nova-drivers"))
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .filter(|e| {
                        e.file_name()
                            .to_string_lossy()
                            .contains(&format!("{}-", std::process::id()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            leftovers.is_empty(),
            "temp copies left behind: {leftovers:?}"
        );
        let _ = std::fs::remove_file(&fake);
    }

    #[test]
    fn abi_version_is_stable_const() {
        assert_eq!(NATIVE_ABI_VERSION, 1);
    }
}
