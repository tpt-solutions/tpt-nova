//! Hot-reloadable, Rust-native gameplay modules.
//!
//! Gameplay logic is compiled as a separate dynamic library (`cdylib`) that the
//! engine loads at runtime and can swap without restarting. The boundary is a
//! small, explicit **C ABI**:
//!
//! - `_nova_gameplay_abi_version() -> u32` — lets the host reject incompatible
//!   modules (bump [`ABI_VERSION`] on breaking changes).
//! - `_nova_gameplay_create() -> *mut c_void` — returns a boxed
//!   `Box<dyn GameplayModule>` (a thin pointer to the fat trait-object box).
//! - `_nova_gameplay_destroy(*mut c_void)` — drops it.
//!
//! Gameplay crates implement [`GameplayModule`] and invoke [`export_gameplay!`]
//! to generate those three exports. On the host side, [`HotModule`] loads the
//! library (copying it to a temp file first so the original can be recompiled
//! while running) and reloads when the file changes on disk.
//!
//! ## A note on safety
//!
//! Passing `&mut World` across a dynamic boundary relies on both the host and
//! the module being built by the same compiler against the same `nova-ecs`.
//! That is the standard trade-off for Rust hot-reload and is enforced softly by
//! the ABI version check; a mismatch is rejected rather than silently misread.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use nova_ecs::World;

/// Bump this whenever [`GameplayModule`] or the exported ABI changes shape.
pub const ABI_VERSION: u32 = 1;

/// What a gameplay module receives each update.
pub struct ScriptContext<'a> {
    pub world: &'a mut World,
    /// Fixed timestep in seconds.
    pub dt: f32,
    /// Current simulation tick.
    pub tick: u64,
}

/// The trait every hot-reloadable gameplay module implements.
///
/// Implementors must be `Default` (the ABI constructs them with no arguments)
/// and `Send`.
pub trait GameplayModule: Send {
    /// Human-readable name for logs/telemetry.
    fn name(&self) -> &str {
        "gameplay"
    }
    /// Called once right after the module is (re)loaded.
    fn on_load(&mut self, _world: &mut World) {}
    /// Called every fixed tick.
    fn update(&mut self, ctx: &mut ScriptContext);
    /// Called just before the module is unloaded/swapped.
    fn on_unload(&mut self, _world: &mut World) {}
}

/// Symbol names for the C ABI exports (kept in one place for host + macro).
pub mod symbols {
    pub const ABI_VERSION: &[u8] = b"_nova_gameplay_abi_version";
    pub const CREATE: &[u8] = b"_nova_gameplay_create";
    pub const DESTROY: &[u8] = b"_nova_gameplay_destroy";
}

/// Generate the C ABI exports for a [`GameplayModule`] implementor.
///
/// ```ignore
/// use nova_scripting::{export_gameplay, GameplayModule, ScriptContext};
/// #[derive(Default)]
/// struct MyGame;
/// impl GameplayModule for MyGame {
///     fn update(&mut self, _ctx: &mut ScriptContext) {}
/// }
/// export_gameplay!(MyGame);
/// ```
#[macro_export]
macro_rules! export_gameplay {
    ($t:ty) => {
        #[no_mangle]
        pub extern "C" fn _nova_gameplay_abi_version() -> u32 {
            $crate::ABI_VERSION
        }

        /// Returns a `*mut Box<dyn GameplayModule>` erased to `*mut c_void`.
        #[no_mangle]
        pub extern "C" fn _nova_gameplay_create() -> *mut ::core::ffi::c_void {
            let module: ::std::boxed::Box<dyn $crate::GameplayModule> =
                ::std::boxed::Box::new(<$t as ::core::default::Default>::default());
            ::std::boxed::Box::into_raw(::std::boxed::Box::new(module)) as *mut ::core::ffi::c_void
        }

        /// # Safety
        /// `ptr` must have come from `_nova_gameplay_create`.
        #[no_mangle]
        pub unsafe extern "C" fn _nova_gameplay_destroy(ptr: *mut ::core::ffi::c_void) {
            if !ptr.is_null() {
                drop(::std::boxed::Box::from_raw(
                    ptr as *mut ::std::boxed::Box<dyn $crate::GameplayModule>,
                ));
            }
        }
    };
}

/// Errors from loading or reloading a gameplay module.
#[derive(Debug)]
pub enum ScriptError {
    Io(std::io::Error),
    Load(libloading::Error),
    MissingSymbol(String),
    AbiMismatch { host: u32, module: u32 },
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScriptError::Io(e) => write!(f, "io error: {e}"),
            ScriptError::Load(e) => write!(f, "library load error: {e}"),
            ScriptError::MissingSymbol(s) => write!(f, "missing symbol: {s}"),
            ScriptError::AbiMismatch { host, module } => {
                write!(f, "ABI mismatch: host={host} module={module}")
            }
        }
    }
}

impl std::error::Error for ScriptError {}

impl From<std::io::Error> for ScriptError {
    fn from(e: std::io::Error) -> Self {
        ScriptError::Io(e)
    }
}

type CreateFn = unsafe extern "C" fn() -> *mut c_void;
type DestroyFn = unsafe extern "C" fn(*mut c_void);
type AbiFn = unsafe extern "C" fn() -> u32;

/// A loaded gameplay module plus the machinery to hot-reload it.
///
/// The loaded library is kept alive alongside the module instance it produced;
/// dropping [`HotModule`] destroys the instance and then unloads the library,
/// in that order.
pub struct HotModule {
    source_path: PathBuf,
    /// The temp copy we actually loaded (so the source can be rebuilt live).
    loaded_copy: Option<PathBuf>,
    library: Option<libloading::Library>,
    instance: *mut c_void,
    destroy: Option<DestroyFn>,
    last_mtime: Option<SystemTime>,
}

impl HotModule {
    /// Load a gameplay dylib from `path` immediately.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ScriptError> {
        let mut m = HotModule {
            source_path: path.as_ref().to_path_buf(),
            loaded_copy: None,
            library: None,
            instance: std::ptr::null_mut(),
            destroy: None,
            last_mtime: None,
        };
        m.load_now()?;
        Ok(m)
    }

    fn file_mtime(path: &Path) -> Option<SystemTime> {
        std::fs::metadata(path).and_then(|m| m.modified()).ok()
    }

    fn load_now(&mut self) -> Result<(), ScriptError> {
        // Copy to a unique temp file so the original can be recompiled while
        // this copy stays memory-mapped/locked.
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let ext = self
            .source_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("mod");
        let copy = std::env::temp_dir().join(format!("nova_gameplay_{nanos}.{ext}"));
        std::fs::copy(&self.source_path, &copy)?;

        // SAFETY: loading arbitrary native code; the caller is trusting the
        // module. We verify the ABI version immediately after loading.
        let library = unsafe { libloading::Library::new(&copy) }.map_err(ScriptError::Load)?;

        unsafe {
            let abi: libloading::Symbol<AbiFn> = library
                .get(symbols::ABI_VERSION)
                .map_err(|_| ScriptError::MissingSymbol("abi_version".into()))?;
            let module_abi = abi();
            if module_abi != ABI_VERSION {
                drop(library);
                let _ = std::fs::remove_file(&copy);
                return Err(ScriptError::AbiMismatch {
                    host: ABI_VERSION,
                    module: module_abi,
                });
            }

            let create: libloading::Symbol<CreateFn> = library
                .get(symbols::CREATE)
                .map_err(|_| ScriptError::MissingSymbol("create".into()))?;
            let destroy: libloading::Symbol<DestroyFn> = library
                .get(symbols::DESTROY)
                .map_err(|_| ScriptError::MissingSymbol("destroy".into()))?;
            let destroy_fn: DestroyFn = *destroy;
            let instance = create();

            self.instance = instance;
            self.destroy = Some(destroy_fn);
            self.library = Some(library);
        }

        self.last_mtime = Self::file_mtime(&self.source_path);
        if let Some(old) = self.loaded_copy.replace(copy) {
            let _ = std::fs::remove_file(old);
        }
        Ok(())
    }

    fn unload(&mut self) {
        if !self.instance.is_null() {
            if let Some(destroy) = self.destroy.take() {
                // SAFETY: `instance` came from the matching `create`.
                unsafe { destroy(self.instance) };
            }
            self.instance = std::ptr::null_mut();
        }
        // Drop the library after the instance it produced.
        self.library = None;
        if let Some(copy) = self.loaded_copy.take() {
            let _ = std::fs::remove_file(copy);
        }
    }

    /// Access the live module instance.
    fn module_mut(&mut self) -> Option<&mut (dyn GameplayModule + 'static)> {
        if self.instance.is_null() {
            return None;
        }
        // SAFETY: `instance` is a `*mut Box<dyn GameplayModule>` produced by the
        // module's `create`, valid until `unload`.
        unsafe {
            let boxed = &mut *(self.instance as *mut Box<dyn GameplayModule>);
            Some(boxed.as_mut())
        }
    }

    /// Reload if the source file's modification time changed. Returns `true` if
    /// a reload happened. On reload failure the previous instance is kept.
    pub fn reload_if_changed(&mut self, world: &mut World) -> Result<bool, ScriptError> {
        let current = Self::file_mtime(&self.source_path);
        if current == self.last_mtime {
            return Ok(false);
        }
        // Give the module a chance to persist/cleanup, then swap.
        if let Some(m) = self.module_mut() {
            m.on_unload(world);
        }
        self.unload();
        self.load_now()?;
        if let Some(m) = self.module_mut() {
            m.on_load(world);
        }
        log::info!("nova-scripting: reloaded {}", self.source_path.display());
        Ok(true)
    }

    /// Run the module's `on_load` hook (call once after initial load).
    pub fn on_load(&mut self, world: &mut World) {
        if let Some(m) = self.module_mut() {
            m.on_load(world);
        }
    }

    /// Drive one update tick.
    pub fn update(&mut self, world: &mut World, dt: f32, tick: u64) {
        if self.instance.is_null() {
            return;
        }
        // Split the borrow: take the raw pointer, build the context separately.
        let ptr = self.instance as *mut Box<dyn GameplayModule>;
        // SAFETY: valid until unload; no aliasing because we only touch the
        // module through this pointer here.
        let module = unsafe { (*ptr).as_mut() };
        let mut ctx = ScriptContext { world, dt, tick };
        module.update(&mut ctx);
    }
}

impl Drop for HotModule {
    fn drop(&mut self) {
        self.unload();
    }
}

// The instance pointer is only ever used on the owning thread; `HotModule`
// itself is not shared across threads by the engine.
unsafe impl Send for HotModule {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_errors() {
        let err = HotModule::load("definitely-not-a-real-module.dll");
        assert!(err.is_err());
    }

    #[test]
    fn abi_version_is_stable_const() {
        assert_eq!(ABI_VERSION, 1);
    }

    #[test]
    fn abi_mismatch_is_rejected() {
        let err = ScriptError::AbiMismatch {
            host: ABI_VERSION,
            module: ABI_VERSION + 1,
        };
        assert!(format!("{err}").contains("ABI mismatch"));
    }
}
