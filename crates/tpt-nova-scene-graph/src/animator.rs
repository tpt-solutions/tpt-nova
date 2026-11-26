//! High-level animation playback over [`tpt-nova-anim`](tpt_nova_anim).
//!
//! [`Animator`] wraps an [`AnimationGraph`] with the calls a scene author
//! actually makes: name a clip, `play` it, `crossfade` to it, `tick` the clock,
//! and read back skinning matrices. It is a plain value with no hidden state,
//! so it can be stored, cloned and inspected like the rest of the scene graph.

use std::collections::{HashMap, HashSet};

use glam::Mat4;
use tpt_nova_anim::{
    compute_skinning, AnimState, AnimTransition, AnimationClip, AnimationGraph, Skeleton,
};

/// Drives one skeleton through named clip states.
#[derive(Debug, Clone)]
pub struct Animator {
    graph: AnimationGraph,
    /// Names of every registered state, including the start state.
    states: HashSet<String>,
    /// Edges already registered with the graph, keyed by `(from, to)`. The
    /// graph uses the first matching edge, so each pair keeps the duration it
    /// was first given.
    edges: HashMap<(String, String), f32>,
    default_fade: f32,
}

impl Animator {
    /// Create an animator that starts in `start` and plays `clips[clip]` for it.
    /// Returns `None` if `clip` is out of range.
    pub fn new(
        skeleton: Skeleton,
        clips: Vec<AnimationClip>,
        start: &str,
        clip: usize,
    ) -> Option<Self> {
        if clip >= clips.len() {
            return None;
        }
        let mut graph = AnimationGraph::new(skeleton, clips, start);
        graph.add_state(AnimState {
            name: start.to_string(),
            clip,
            speed: 1.0,
            loop_: true,
        });
        Some(Animator {
            graph,
            states: HashSet::from([start.to_string()]),
            edges: HashMap::new(),
            default_fade: 0.25,
        })
    }

    /// Fade time used by [`Animator::crossfade_default`] (seconds).
    pub fn with_default_fade(mut self, seconds: f32) -> Self {
        self.default_fade = seconds.max(0.0);
        self
    }

    /// Register a named state that plays `clips[clip]`. Returns false if the
    /// clip index is out of range or the name is already taken.
    pub fn add_state(&mut self, name: &str, clip: usize, looping: bool) -> bool {
        if clip >= self.graph.clips.len() || !self.states.insert(name.to_string()) {
            return false;
        }
        self.graph.add_state(AnimState {
            name: name.to_string(),
            clip,
            speed: 1.0,
            loop_: looping,
        });
        true
    }

    /// Hard-cut to a state, restarting its clock. Cancels any blend in progress.
    pub fn play(&mut self, state: &str) -> bool {
        self.graph.jump_to(state)
    }

    /// Blend to `state` over `seconds`. The first crossfade for a given pair
    /// fixes that pair's duration. Returns false if a blend is already running,
    /// the state is unknown, or it is the current state.
    pub fn crossfade(&mut self, state: &str, seconds: f32) -> bool {
        if self.graph.is_blending() || state == self.graph.current_state() {
            return false;
        }
        let from = self.graph.current_state().to_string();
        let key = (from.clone(), state.to_string());
        if !self.edges.contains_key(&key) {
            self.graph.add_transition(AnimTransition {
                from,
                to: state.to_string(),
                duration: seconds.max(0.0),
            });
            self.edges.insert(key, seconds.max(0.0));
        }
        self.graph.transition_to(state)
    }

    /// [`Animator::crossfade`] using the configured default fade time.
    pub fn crossfade_default(&mut self, state: &str) -> bool {
        let fade = self.default_fade;
        self.crossfade(state, fade)
    }

    /// Advance playback by `dt` seconds.
    pub fn tick(&mut self, dt: f32) {
        self.graph.update(dt);
    }

    /// The state playback is in (the source state while blending).
    pub fn current(&self) -> &str {
        self.graph.current_state()
    }

    pub fn is_blending(&self) -> bool {
        self.graph.is_blending()
    }

    /// Skinning matrices for the current (possibly blended) pose, one per bone.
    pub fn skin_matrices(&self) -> Vec<Mat4> {
        compute_skinning(&self.graph.skeleton, &self.graph.pose())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Quat, Vec3};
    use tpt_nova_anim::{Bone, Channel};

    fn skeleton() -> Skeleton {
        Skeleton {
            bones: vec![Bone::new("root", None, Mat4::IDENTITY)],
        }
        .finalize()
    }

    /// A one-bone clip that rises linearly to `height` over one second.
    fn rise(height: f32) -> AnimationClip {
        AnimationClip {
            name: format!("rise-{height}"),
            duration: 1.0,
            channels: vec![Channel::new(
                0,
                vec![0.0, 1.0],
                vec![Vec3::ZERO, Vec3::new(0.0, height, 0.0)],
                vec![Quat::IDENTITY, Quat::IDENTITY],
                vec![Vec3::ONE, Vec3::ONE],
            )],
        }
    }

    fn animator() -> Animator {
        let mut a = Animator::new(skeleton(), vec![rise(1.0), rise(3.0)], "idle", 0).unwrap();
        assert!(a.add_state("run", 1, true));
        a
    }

    fn translation_y(a: &Animator) -> f32 {
        // Skin matrix for a root bone with identity inverse-bind is the pose itself.
        a.skin_matrices()[0].w_axis.y
    }

    #[test]
    fn rejects_out_of_range_clips_and_duplicate_states() {
        assert!(Animator::new(skeleton(), vec![rise(1.0)], "idle", 5).is_none());
        let mut a = animator();
        assert!(!a.add_state("run", 1, true), "duplicate name");
        assert!(!a.add_state("bad", 9, true), "clip out of range");
    }

    #[test]
    fn play_cuts_and_tick_advances_the_clock() {
        let mut a = animator();
        a.tick(0.5);
        assert!((translation_y(&a) - 0.5).abs() < 1e-4);
        assert!(a.play("run"));
        assert_eq!(a.current(), "run");
        assert!(translation_y(&a).abs() < 1e-4, "play restarts the clock");
        assert!(!a.play("missing"));
    }

    #[test]
    fn crossfade_needs_an_edge_and_blends_to_the_target() {
        let mut a = animator();
        assert!(a.crossfade("run", 1.0));
        assert!(a.is_blending());
        assert!(!a.crossfade("idle", 1.0), "no second blend mid-transition");
        a.tick(1.0);
        assert!(!a.is_blending());
        assert_eq!(a.current(), "run");
    }

    #[test]
    fn first_crossfade_duration_is_kept_for_the_pair() {
        let mut a = animator();
        assert!(a.crossfade("run", 2.0));
        a.tick(2.0);
        assert!(a.crossfade("idle", 0.1), "reverse edge is new");
        a.tick(0.1);
        // Re-using the first pair with a different duration keeps the original.
        assert!(a.crossfade("run", 0.1));
        a.tick(0.5);
        assert!(a.is_blending(), "still blending: duration stayed at 2s");
    }

    #[test]
    fn crossfade_to_current_state_is_refused() {
        let mut a = animator();
        assert!(!a.crossfade("idle", 0.5));
        assert!(!a.is_blending());
    }

    #[test]
    fn default_fade_is_used_by_crossfade_default() {
        let mut a = animator().with_default_fade(0.5);
        assert!(a.crossfade_default("run"));
        a.tick(0.5);
        assert!(!a.is_blending());
    }
}
