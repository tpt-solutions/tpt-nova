//! Declarative post-processing stack.
//!
//! This is the serializable, scene-owned description of the effects. It holds
//! no GPU state: [`PostProcessPipeline::to_render_effects`] lowers it into the
//! `tpt-nova-render` [`PostProcessChain`](tpt_nova_render::post::PostProcessChain)
//! effect list, where the renderer owns the actual pipelines.

use serde::{Deserialize, Serialize};
use tpt_nova_render::post::{BloomSettings, Effect, ToneMapping};

/// Which effect a [`PostProcessEffect`] is, without its parameters. The pipeline
/// holds at most one effect per kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PostEffectKind {
    Bloom,
    ToneMapping,
    Fxaa,
}

/// Tone-mapping curve choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToneCurve {
    /// Filmic ACES fit (default).
    Aces,
    /// Simple Reinhard.
    Reinhard,
}

/// One effect in the scene's post stack, with its authored parameters.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PostProcessEffect {
    /// Glow from pixels brighter than `threshold`, blended in at `intensity`.
    Bloom { threshold: f32, intensity: f32 },
    /// HDR → LDR mapping.
    ToneMapping { curve: ToneCurve, exposure: f32 },
    /// Luma-edge anti-aliasing.
    Fxaa,
}

impl PostProcessEffect {
    /// The default bloom (threshold 0.8, intensity 0.6).
    pub fn bloom() -> Self {
        let d = BloomSettings::default();
        PostProcessEffect::Bloom {
            threshold: d.threshold,
            intensity: d.intensity,
        }
    }

    /// ACES tone mapping at exposure 1.0.
    pub fn tone_mapping() -> Self {
        PostProcessEffect::ToneMapping {
            curve: ToneCurve::Aces,
            exposure: 1.0,
        }
    }

    pub fn kind(&self) -> PostEffectKind {
        match self {
            PostProcessEffect::Bloom { .. } => PostEffectKind::Bloom,
            PostProcessEffect::ToneMapping { .. } => PostEffectKind::ToneMapping,
            PostProcessEffect::Fxaa => PostEffectKind::Fxaa,
        }
    }

    /// Lower this effect into the renderer's effect description.
    pub fn to_render(&self) -> Effect {
        match *self {
            PostProcessEffect::Bloom {
                threshold,
                intensity,
            } => Effect::Bloom(BloomSettings {
                threshold,
                intensity,
            }),
            PostProcessEffect::ToneMapping { curve, exposure } => {
                let mapping = match curve {
                    ToneCurve::Aces => ToneMapping::aces(),
                    ToneCurve::Reinhard => ToneMapping::reinhard(),
                };
                Effect::ToneMapping(mapping.with_exposure(exposure))
            }
            PostProcessEffect::Fxaa => Effect::Fxaa,
        }
    }
}

/// An ordered stack of at most one effect per [`PostEffectKind`].
///
/// Effects run in list order, so the order is significant (e.g. tone mapping
/// should come after bloom so the glow is mapped too).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PostProcessPipeline {
    effects: Vec<PostProcessEffect>,
}

impl PostProcessPipeline {
    /// No effects at all.
    pub fn empty() -> Self {
        Self::default()
    }

    /// The "stunning defaults" stack: bloom → ACES tone mapping → FXAA.
    pub fn stunning_defaults() -> Self {
        PostProcessPipeline {
            effects: vec![
                PostProcessEffect::bloom(),
                PostProcessEffect::tone_mapping(),
                PostProcessEffect::Fxaa,
            ],
        }
    }

    /// Enable `effect`. If an effect of the same kind is already present its
    /// parameters are replaced in place, keeping its position in the stack.
    pub fn enable(&mut self, effect: PostProcessEffect) {
        match self.effects.iter_mut().find(|e| e.kind() == effect.kind()) {
            Some(slot) => *slot = effect,
            None => self.effects.push(effect),
        }
    }

    /// Remove the effect of `kind`. Returns `false` if it was not enabled.
    pub fn disable(&mut self, kind: PostEffectKind) -> bool {
        let before = self.effects.len();
        self.effects.retain(|e| e.kind() != kind);
        self.effects.len() != before
    }

    pub fn is_enabled(&self, kind: PostEffectKind) -> bool {
        self.effects.iter().any(|e| e.kind() == kind)
    }

    /// The enabled effects, in run order.
    pub fn effects(&self) -> &[PostProcessEffect] {
        &self.effects
    }

    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }

    /// Lower the whole stack for `tpt-nova-render`'s `PostProcessChain::set_effects`.
    pub fn to_render_effects(&self) -> Vec<Effect> {
        self.effects
            .iter()
            .map(PostProcessEffect::to_render)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_run_bloom_then_tonemap_then_fxaa() {
        let p = PostProcessPipeline::stunning_defaults();
        let kinds: Vec<_> = p.effects().iter().map(|e| e.kind()).collect();
        assert_eq!(
            kinds,
            vec![
                PostEffectKind::Bloom,
                PostEffectKind::ToneMapping,
                PostEffectKind::Fxaa
            ]
        );
    }

    #[test]
    fn enable_replaces_same_kind_in_place() {
        let mut p = PostProcessPipeline::stunning_defaults();
        p.enable(PostProcessEffect::Bloom {
            threshold: 0.5,
            intensity: 1.0,
        });
        assert_eq!(p.effects().len(), 3);
        assert_eq!(p.effects()[0].kind(), PostEffectKind::Bloom);
        assert_eq!(
            p.effects()[0],
            PostProcessEffect::Bloom {
                threshold: 0.5,
                intensity: 1.0
            }
        );
    }

    #[test]
    fn disable_reports_whether_it_was_enabled() {
        let mut p = PostProcessPipeline::stunning_defaults();
        assert!(p.disable(PostEffectKind::Fxaa));
        assert!(!p.disable(PostEffectKind::Fxaa));
        assert!(!p.is_enabled(PostEffectKind::Fxaa));
        assert!(PostProcessPipeline::empty().is_empty());
    }

    #[test]
    fn lowering_preserves_parameters() {
        let p = PostProcessPipeline {
            effects: vec![
                PostProcessEffect::Bloom {
                    threshold: 0.7,
                    intensity: 0.2,
                },
                PostProcessEffect::ToneMapping {
                    curve: ToneCurve::Reinhard,
                    exposure: 2.0,
                },
            ],
        };
        let lowered = p.to_render_effects();
        assert_eq!(
            lowered,
            vec![
                Effect::Bloom(BloomSettings {
                    threshold: 0.7,
                    intensity: 0.2
                }),
                Effect::ToneMapping(ToneMapping::reinhard().with_exposure(2.0)),
            ]
        );
    }
}
