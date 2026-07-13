//! Skeletal animation for TPT Nova.
//!
//! The crate is split into pure, fully-testable math and a thin ECS layer:
//!
//! - [`Bone`]/[`Skeleton`] — the rest pose and inverse-bind matrices.
//! - [`AnimationClip`]/[`Channel`] — keyframed tracks per bone.
//! - [`sample_clip`] / [`compute_skinning`] — turn time into skinning matrices.
//! - [`blend_poses`] — cross-fade between two sampled poses.
//! - [`AnimationGraph`] — a tiny finite-state machine (states + transitions)
//!   that drives blends between clips.
//! - [`Animated`] + [`step_animation`] — the ECS-facing component/system that
//!   writes [`SkinMatrices`] for the renderer to consume.

use glam::{Mat4, Quat, Vec3};

use nova_ecs::component::Component;
use nova_ecs::World;

/// A local transform for a single bone at a given time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BonePose {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl BonePose {
    pub fn identity() -> Self {
        BonePose {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }

    /// Compose into a local bone matrix.
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

/// One bone in a [`Skeleton`].
#[derive(Debug, Clone, PartialEq)]
pub struct Bone {
    pub name: String,
    /// Parent bone index, or `None` for a root.
    pub parent: Option<usize>,
    /// Inverse of the bone's world-space rest transform (bind pose).
    pub inverse_bind_pose: Mat4,
    /// Local rest transform (identity-pose basis for channels with no track).
    pub rest_local: Mat4,
}

impl Bone {
    /// Convenience for building a bone whose rest local transform is given.
    pub fn new(name: impl Into<String>, parent: Option<usize>, rest_local: Mat4) -> Self {
        Bone {
            name: name.into(),
            parent,
            inverse_bind_pose: Mat4::IDENTITY,
            rest_local,
        }
    }

    fn rest_pose(&self) -> BonePose {
        let (scale, rotation, translation) = self.rest_local.to_scale_rotation_translation();
        BonePose {
            translation,
            rotation,
            scale,
        }
    }
}

/// A hierarchy of bones with their inverse-bind matrices.
#[derive(Debug, Clone, PartialEq)]
pub struct Skeleton {
    pub bones: Vec<Bone>,
}

impl Skeleton {
    /// Build a skeleton and bake inverse-bind matrices from each bone's world
    /// rest transform. Call once after constructing [`Bone`]s with their
    /// `rest_local` transforms.
    pub fn finalize(mut self) -> Self {
        let n = self.bones.len();
        let mut world = vec![Mat4::IDENTITY; n];
        for i in 0..n {
            let local = self.bones[i].rest_local;
            let parent_world = match self.bones[i].parent {
                Some(p) => world[p],
                None => Mat4::IDENTITY,
            };
            let w = parent_world * local;
            world[i] = w;
            self.bones[i].inverse_bind_pose = w.inverse();
        }
        self
    }
}

/// A single bone's keyframed track. All four arrays share the same `times`
/// index spacing.
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    pub bone: usize,
    pub times: Vec<f32>,
    pub translations: Vec<Vec3>,
    pub rotations: Vec<Quat>,
    pub scales: Vec<Vec3>,
}

impl Channel {
    pub fn new(
        bone: usize,
        times: Vec<f32>,
        translations: Vec<Vec3>,
        rotations: Vec<Quat>,
        scales: Vec<Vec3>,
    ) -> Self {
        Channel {
            bone,
            times,
            translations,
            rotations,
            scales,
        }
    }
}

/// A named, time-bounded animation (seconds).
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationClip {
    pub name: String,
    pub duration: f32,
    pub channels: Vec<Channel>,
}

/// Sample a clip at `time` into one [`BonePose`] per bone of `skeleton`.
/// Bones without a channel use their rest pose.
pub fn sample_clip(clip: &AnimationClip, skeleton: &Skeleton, time: f32) -> Vec<BonePose> {
    let mut poses = Vec::with_capacity(skeleton.bones.len());
    for (i, bone) in skeleton.bones.iter().enumerate() {
        let ch = clip.channels.iter().find(|c| c.bone == i);
        let pose = match ch {
            Some(c) => BonePose {
                translation: sample_vec3(&c.times, &c.translations, time),
                rotation: sample_quat(&c.times, &c.rotations, time),
                scale: sample_vec3(&c.times, &c.scales, time),
            },
            None => bone.rest_pose(),
        };
        poses.push(pose);
    }
    poses
}

/// Multiply local poses up the hierarchy and offset by inverse-bind matrices,
/// yielding the skinning matrices a vertex shader multiplies skinned vertices
/// by (one per bone, matching [`Skeleton::bones`] order).
pub fn compute_skinning(skeleton: &Skeleton, local: &[BonePose]) -> Vec<Mat4> {
    let n = skeleton.bones.len();
    let mut world = vec![Mat4::IDENTITY; n];
    let mut skin = vec![Mat4::IDENTITY; n];
    for i in 0..n {
        let local_mat = local[i].matrix();
        let parent_world = match skeleton.bones[i].parent {
            Some(p) => world[p],
            None => Mat4::IDENTITY,
        };
        let w = parent_world * local_mat;
        world[i] = w;
        skin[i] = w * skeleton.bones[i].inverse_bind_pose;
    }
    skin
}

/// Cross-fade two sampled poses. `t == 0` returns `a`, `t == 1` returns `b`.
pub fn blend_poses(a: &[BonePose], b: &[BonePose], t: f32) -> Vec<BonePose> {
    let t = t.clamp(0.0, 1.0);
    a.iter()
        .zip(b.iter())
        .map(|(pa, pb)| BonePose {
            translation: pa.translation.lerp(pb.translation, t),
            rotation: pa.rotation.slerp(pb.rotation, t),
            scale: pa.scale.lerp(pb.scale, t),
        })
        .collect()
}

// ---- Keyframe interpolation helpers --------------------------------------

fn locate(times: &[f32], t: f32) -> (usize, usize, f32) {
    if times.is_empty() {
        return (0, 0, 0.0);
    }
    if t <= times[0] {
        return (0, 0, 0.0);
    }
    let last = times.len() - 1;
    if t >= times[last] {
        return (last, last, 0.0);
    }
    let mut i = 0;
    while i < last && times[i + 1] < t {
        i += 1;
    }
    let i1 = i + 1;
    let span = times[i1] - times[i];
    let frac = if span > 0.0 {
        (t - times[i]) / span
    } else {
        0.0
    };
    (i, i1, frac)
}

fn sample_vec3(times: &[f32], values: &[Vec3], t: f32) -> Vec3 {
    if values.is_empty() {
        return Vec3::ZERO;
    }
    let (i0, i1, f) = locate(times, t);
    values[i0].lerp(values[i1], f)
}

fn sample_quat(times: &[f32], values: &[Quat], t: f32) -> Quat {
    if values.is_empty() {
        return Quat::IDENTITY;
    }
    let (i0, i1, f) = locate(times, t);
    values[i0].slerp(values[i1], f)
}

// ---- Animation state machine ---------------------------------------------

/// A single state in the [`AnimationGraph`]: one clip, a playback speed, and a
/// loop flag.
#[derive(Debug, Clone)]
pub struct AnimState {
    pub name: String,
    pub clip: usize,
    pub speed: f32,
    pub loop_: bool,
}

/// A transition between two states, blended over `duration` seconds.
#[derive(Debug, Clone)]
pub struct AnimTransition {
    pub from: String,
    pub to: String,
    pub duration: f32,
}

/// A minimal finite-state machine driving animation playback.
///
/// Holds the clips and skeleton so it can both advance time and produce the
/// resolved (possibly blended) pose in one place.
#[derive(Debug, Clone)]
pub struct AnimationGraph {
    pub skeleton: Skeleton,
    pub clips: Vec<AnimationClip>,
    states: Vec<AnimState>,
    transitions: Vec<AnimTransition>,
    current: String,
    current_time: f32,
    blending: bool,
    blend: f32,
    target: String,
    target_time: f32,
}

impl AnimationGraph {
    pub fn new(skeleton: Skeleton, clips: Vec<AnimationClip>, start: &str) -> Self {
        AnimationGraph {
            skeleton,
            clips,
            states: Vec::new(),
            transitions: Vec::new(),
            current: start.to_string(),
            current_time: 0.0,
            blending: false,
            blend: 0.0,
            target: start.to_string(),
            target_time: 0.0,
        }
    }

    pub fn add_state(&mut self, state: AnimState) {
        self.states.push(state);
    }

    pub fn add_transition(&mut self, transition: AnimTransition) {
        self.transitions.push(transition);
    }

    /// Begin a transition to `to` if a matching transition edge exists and we
    /// are not already blending. Returns true if a transition started.
    pub fn transition_to(&mut self, to: &str) -> bool {
        if self.blending || to == self.current {
            return false;
        }
        let ok = self
            .transitions
            .iter()
            .any(|t| t.from == self.current && t.to == to);
        if !ok {
            return false;
        }
        self.target = to.to_string();
        self.target_time = 0.0;
        self.blending = true;
        self.blend = 0.0;
        true
    }

    /// Advance both state clocks by `dt` seconds.
    pub fn update(&mut self, dt: f32) {
        if let Some(s) = self.state(&self.current) {
            self.current_time = advance(
                self.current_time,
                s.speed * dt,
                s.loop_,
                self.clip_duration(s.clip),
            );
        }
        if self.blending {
            if let Some(s) = self.state(&self.target) {
                self.target_time = advance(
                    self.target_time,
                    s.speed * dt,
                    s.loop_,
                    self.clip_duration(s.clip),
                );
            }
            let dur = self
                .transitions
                .iter()
                .find(|t| t.from == self.current && t.to == self.target)
                .map(|t| t.duration)
                .unwrap_or(0.0);
            self.blend = if dur <= 0.0 {
                1.0
            } else {
                (self.blend + dt / dur).clamp(0.0, 1.0)
            };
            if self.blend >= 1.0 {
                self.current = self.target.clone();
                self.current_time = self.target_time;
                self.blending = false;
            }
        }
    }

    fn state(&self, name: &str) -> Option<&AnimState> {
        self.states.iter().find(|s| s.name == name)
    }

    fn clip_duration(&self, clip: usize) -> f32 {
        self.clips.get(clip).map(|c| c.duration).unwrap_or(0.0)
    }

    /// Resolve the current (possibly blended) local pose for every bone.
    pub fn pose(&self) -> Vec<BonePose> {
        let s_cur = match self.state(&self.current) {
            Some(s) => s,
            None => return vec![BonePose::identity(); self.skeleton.bones.len()],
        };
        let cur_clip = match self.clips.get(s_cur.clip) {
            Some(c) => c,
            None => return vec![BonePose::identity(); self.skeleton.bones.len()],
        };
        let cur = sample_clip(cur_clip, &self.skeleton, self.current_time);

        if !self.blending {
            return cur;
        }
        let s_tgt = match self.state(&self.target) {
            Some(s) => s,
            None => return cur,
        };
        let tgt_clip = match self.clips.get(s_tgt.clip) {
            Some(c) => c,
            None => return cur,
        };
        let tgt = sample_clip(tgt_clip, &self.skeleton, self.target_time);
        // Smoothstep the blend for a less robotic cross-fade.
        let t = self.blend * self.blend * (3.0 - 2.0 * self.blend);
        blend_poses(&cur, &tgt, t)
    }
}

fn advance(time: f32, delta: f32, loop_: bool, duration: f32) -> f32 {
    if duration <= 0.0 {
        return 0.0;
    }
    let mut t = time + delta;
    if t >= duration {
        if loop_ {
            t %= duration;
        } else {
            t = duration;
        }
    }
    t.max(0.0)
}

// ---- ECS layer ------------------------------------------------------------

/// Skinning matrices produced by [`step_animation`], one per bone, in skeleton
/// order. The renderer uploads these to the vertex shader's bone palette.
#[derive(Debug, Clone, PartialEq)]
pub struct SkinMatrices(pub Vec<Mat4>);

/// An entity that plays skeletal animation via an [`AnimationGraph`].
#[derive(Debug, Clone)]
pub struct Animated {
    pub graph: AnimationGraph,
}

impl Component for SkinMatrices {}
impl Component for Animated {}

/// Advance every [`Animated`] entity and write its [`SkinMatrices`].
pub fn step_animation(world: &mut World, dt: f32) {
    let entities: Vec<_> = world
        .query_1::<Animated>()
        .into_iter()
        .map(|(e, _)| e)
        .collect();
    for e in entities {
        let (poses, skeleton) = {
            let anim = match world.get_component_mut::<Animated>(e) {
                Some(a) => a,
                None => continue,
            };
            anim.graph.update(dt);
            (anim.graph.pose(), anim.graph.skeleton.clone())
        };
        let skins = compute_skinning(&skeleton, &poses);
        world.add_component(e, SkinMatrices(skins));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn two_bone_skeleton() -> Skeleton {
        Skeleton {
            bones: vec![
                Bone::new("root", None, Mat4::IDENTITY),
                Bone::new("child", Some(0), Mat4::IDENTITY),
            ],
        }
        .finalize()
    }

    fn translate_clip(bone: usize, from: Vec3, to: Vec3, dur: f32) -> AnimationClip {
        AnimationClip {
            name: "move".into(),
            duration: dur,
            channels: vec![Channel::new(
                bone,
                vec![0.0, dur],
                vec![from, to],
                vec![Quat::IDENTITY, Quat::IDENTITY],
                vec![Vec3::ONE, Vec3::ONE],
            )],
        }
    }

    #[test]
    fn sample_clip_lerps_translation() {
        let sk = two_bone_skeleton();
        let clip = translate_clip(0, Vec3::ZERO, Vec3::new(0.0, 10.0, 0.0), 1.0);
        let p = sample_clip(&clip, &sk, 0.5);
        assert!((p[0].translation.y - 5.0).abs() < 1e-4);
    }

    #[test]
    fn sample_clip_uses_rest_pose_without_channel() {
        let sk = two_bone_skeleton();
        let clip = translate_clip(0, Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), 1.0);
        let p = sample_clip(&clip, &sk, 0.0);
        // Bone 1 has no channel -> identity rest pose.
        assert_eq!(p[1], BonePose::identity());
    }

    #[test]
    fn compute_skinning_offsets_by_inverse_bind() {
        // Bone 0 at origin, bone 1 child translated +2 on X in rest.
        let sk = Skeleton {
            bones: vec![
                Bone::new("root", None, Mat4::IDENTITY),
                Bone::new("child", Some(0), Mat4::from_translation(Vec3::X * 2.0)),
            ],
        }
        .finalize();
        // Posing each bone at its rest local transform must yield identity skin.
        let pose = vec![
            BonePose::identity(),
            BonePose {
                translation: Vec3::X * 2.0,
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            },
        ];
        let skins = compute_skinning(&sk, &pose);
        assert!(sk.bones[1]
            .inverse_bind_pose
            .abs_diff_eq(Mat4::from_translation(Vec3::X * -2.0), 1e-5));
        assert!(skins[0].abs_diff_eq(Mat4::IDENTITY, 1e-5));
        assert!(skins[1].abs_diff_eq(Mat4::IDENTITY, 1e-5));
    }

    #[test]
    fn blend_poses_is_midpoint_at_half() {
        let a = vec![BonePose {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }];
        let b = vec![BonePose {
            translation: Vec3::new(0.0, 10.0, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }];
        let m = blend_poses(&a, &b, 0.5);
        assert!((m[0].translation.y - 5.0).abs() < 1e-4);
    }

    #[test]
    fn graph_blends_between_clips_during_transition() {
        let sk = two_bone_skeleton();
        let clip_a = translate_clip(0, Vec3::ZERO, Vec3::new(0.0, 4.0, 0.0), 1.0);
        let clip_b = translate_clip(0, Vec3::ZERO, Vec3::new(0.0, 8.0, 0.0), 1.0);
        let mut g = AnimationGraph::new(sk, vec![clip_a, clip_b], "A");
        g.add_state(AnimState {
            name: "A".into(),
            clip: 0,
            speed: 1.0,
            loop_: false,
        });
        g.add_state(AnimState {
            name: "B".into(),
            clip: 1,
            speed: 1.0,
            loop_: false,
        });
        g.add_transition(AnimTransition {
            from: "A".into(),
            to: "B".into(),
            duration: 1.0,
        });
        assert!(g.transition_to("B"));
        // Advance half the transition; pose should be ~midpoint of both clips
        // at their respective times (both at t=0.5 -> 2.0 and 4.0 -> mid 3.0).
        g.update(0.5);
        let pose = g.pose();
        assert!(
            (pose[0].translation.y - 3.0).abs() < 1e-3,
            "got {}",
            pose[0].translation.y
        );
        assert!(g.blending);
        // Finish the transition: we land fully on clip B at its end (t = 1).
        g.update(0.5);
        assert!(!g.blending);
        let pose = g.pose();
        assert!(
            (pose[0].translation.y - 8.0).abs() < 1e-3,
            "got {}",
            pose[0].translation.y
        );
    }

    #[test]
    fn step_animation_writes_skin_matrices_into_world() {
        let sk = two_bone_skeleton();
        let clip = translate_clip(0, Vec3::ZERO, Vec3::new(0.0, 2.0, 0.0), 1.0);
        let mut g = AnimationGraph::new(sk, vec![clip], "A");
        g.add_state(AnimState {
            name: "A".into(),
            clip: 0,
            speed: 1.0,
            loop_: true,
        });
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Animated { graph: g });

        step_animation(&mut world, 0.5);
        let skins = world.get_component::<SkinMatrices>(e).unwrap();
        assert_eq!(skins.0.len(), 2);
        // Bone 0 moved to y=1, its skin = world(translate y=1) * inv_bind(identity) = translate y=1.
        assert!(skins.0[0].abs_diff_eq(Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0)), 1e-4));
    }

    #[test]
    fn sample_clamps_before_first_and_after_last_keyframe() {
        let sk = two_bone_skeleton();
        let clip = translate_clip(0, Vec3::ZERO, Vec3::new(0.0, 10.0, 0.0), 1.0);
        let early = sample_clip(&clip, &sk, -5.0);
        assert_eq!(early[0].translation, Vec3::ZERO);
        let late = sample_clip(&clip, &sk, 100.0);
        assert!(
            (late[0].translation.y - 10.0).abs() < 1e-4,
            "expected clamp to last keyframe, got {}",
            late[0].translation.y
        );
    }

    #[test]
    fn single_keyframe_clip_is_constant_in_time() {
        let sk = two_bone_skeleton();
        let clip = AnimationClip {
            name: "static".into(),
            duration: 1.0,
            channels: vec![Channel::new(
                0,
                vec![0.0],
                vec![Vec3::new(0.0, 3.0, 0.0)],
                vec![Quat::IDENTITY],
                vec![Vec3::ONE],
            )],
        };
        let a = sample_clip(&clip, &sk, 0.0);
        let b = sample_clip(&clip, &sk, 0.5);
        let c = sample_clip(&clip, &sk, 1.0);
        assert_eq!(a[0].translation, b[0].translation);
        assert_eq!(b[0].translation, c[0].translation);
        assert!((a[0].translation.y - 3.0).abs() < 1e-4);
    }

    #[test]
    fn state_machine_chains_idle_walk_run() {
        let sk = two_bone_skeleton();
        let idle = translate_clip(0, Vec3::ZERO, Vec3::ZERO, 1.0);
        let walk = translate_clip(0, Vec3::ZERO, Vec3::new(0.0, 2.0, 0.0), 1.0);
        let run = translate_clip(0, Vec3::ZERO, Vec3::new(0.0, 5.0, 0.0), 1.0);
        let mut g = AnimationGraph::new(sk, vec![idle, walk, run], "idle");
        for (i, name) in ["idle", "walk", "run"].iter().enumerate() {
            g.add_state(AnimState {
                name: name.to_string(),
                clip: i,
                speed: 1.0,
                loop_: true,
            });
        }
        g.add_transition(AnimTransition {
            from: "idle".into(),
            to: "walk".into(),
            duration: 0.5,
        });
        g.add_transition(AnimTransition {
            from: "walk".into(),
            to: "run".into(),
            duration: 0.5,
        });

        // idle -> walk finishes on clip B, mid-point of its travel.
        assert!(g.transition_to("walk"));
        g.update(0.5);
        assert!(!g.blending, "idle->walk transition should complete");
        assert!((g.pose()[0].translation.y - 1.0).abs() < 1e-3);

        // walk -> run finishes on clip C.
        assert!(g.transition_to("run"));
        g.update(0.5);
        assert!(!g.blending, "walk->run transition should complete");
        assert!((g.pose()[0].translation.y - 2.5).abs() < 1e-3);

        // No run->idle / run->walk edges exist, so these are rejected.
        assert!(!g.transition_to("idle"));
        assert!(!g.transition_to("walk"));

        // A transition that is already in progress cannot be re-triggered.
        assert!(!g.transition_to("idle"));
    }

    #[test]
    fn no_transition_while_blending() {
        let sk = two_bone_skeleton();
        let a = translate_clip(0, Vec3::ZERO, Vec3::new(0.0, 2.0, 0.0), 1.0);
        let b = translate_clip(0, Vec3::ZERO, Vec3::new(0.0, 4.0, 0.0), 1.0);
        let mut g = AnimationGraph::new(sk, vec![a, b], "A");
        g.add_state(AnimState {
            name: "A".into(),
            clip: 0,
            speed: 1.0,
            loop_: true,
        });
        g.add_state(AnimState {
            name: "B".into(),
            clip: 1,
            speed: 1.0,
            loop_: true,
        });
        g.add_transition(AnimTransition {
            from: "A".into(),
            to: "B".into(),
            duration: 1.0,
        });
        assert!(g.transition_to("B"));
        g.update(0.2); // mid-blend
        assert!(g.blending);
        assert!(!g.transition_to("B"));
    }
}
