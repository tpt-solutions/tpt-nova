//! Procedural auto-rigging.
//!
//! A fully automatic character rig is a research problem; for ingestion we use
//! a robust, predictable heuristic: a vertical "spine" of bones spanning the
//! mesh's bounding box. This gives downstream animation systems (blend trees,
//! IK, physics-driven joints) a sane skeleton to attach to without requiring an
//! artist-authored rig. The produced [`Skeleton`](nova_anim::Skeleton) has its
//! inverse-bind matrices baked and is ready for [`nova_anim::compute_skinning`].

use glam::Mat4;
use glam::Vec3;

use crate::loader::Aabb;

/// Build a vertical chain of `segments` bones from the bottom to the top of
/// `bounds`. Bone 0 is the root at the box center-bottom; each subsequent bone
/// is a child offset straight up by one segment length.
pub fn auto_rig(bounds: &Aabb, segments: usize) -> nova_anim::Skeleton {
    let segments = segments.max(2);
    let cx = (bounds.min.x + bounds.max.x) * 0.5;
    let cz = (bounds.min.z + bounds.max.z) * 0.5;
    let min_y = bounds.min.y;
    let max_y = bounds.max.y;
    let seg = (max_y - min_y) / (segments as f32 - 1.0);

    let mut bones = Vec::with_capacity(segments);
    for i in 0..segments {
        let (parent, rest_local) = if i == 0 {
            (None, Mat4::from_translation(Vec3::new(cx, min_y, cz)))
        } else {
            (
                Some(i - 1),
                Mat4::from_translation(Vec3::new(0.0, seg, 0.0)),
            )
        };
        bones.push(nova_anim::Bone::new(
            format!("bone_{i}"),
            parent,
            rest_local,
        ));
    }

    nova_anim::Skeleton { bones }.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_anim::{compute_skinning, BonePose};

    #[test]
    fn auto_rig_builds_expected_bone_count_and_rest_skin() {
        let bounds = Aabb {
            min: Vec3::new(-1.0, 0.0, -1.0),
            max: Vec3::new(1.0, 3.0, 1.0),
        };
        let sk = auto_rig(&bounds, 4);
        assert_eq!(sk.bones.len(), 4);
        // Bone 0 is the root at the box center-bottom.
        assert!(sk.bones[0]
            .rest_local
            .abs_diff_eq(Mat4::from_translation(Vec3::new(0.0, 0.0, 0.0)), 1e-5));
        // Subsequent bones are root children offset upward by 1.0 (3 height / 3).
        assert!(sk.bones[1].parent == Some(0));

        // Skinning the *rest* pose must yield identity matrices. The rest pose
        // is each bone's `rest_local` decomposed into translation/rotation/scale.
        let poses: Vec<BonePose> = sk
            .bones
            .iter()
            .map(|b| {
                let (s, r, t) = b.rest_local.to_scale_rotation_translation();
                BonePose {
                    translation: t,
                    rotation: r,
                    scale: s,
                }
            })
            .collect();
        let skins = compute_skinning(&sk, &poses);
        for s in &skins {
            assert!(s.abs_diff_eq(Mat4::IDENTITY, 1e-5));
        }
    }
}
