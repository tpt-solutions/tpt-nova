//! "Highlight & Fix" viewport overlay tooling.
//!
//! Lets a human (or an AI agent driving the editor) draw a rectangle around a
//! region of the rendered viewport and turn that selection into a structured
//! **fix request** for an LLM: the normalized region bounds, the set of entity
//! ids whose projected centers fall inside it, and a natural-language
//! instruction. The renderer draws the highlight rectangle; this crate owns the
//! *logic* — region math, entity picking, and prompt assembly — which is fully
//! testable without a GPU.
//!
//! The generated [`AiFixRequest`] is exactly the shape an external coding agent
//! (see `nova-agent-api`) consumes: it can read the highlighted entities via
//! telemetry, apply [`nova_agent_api::AgentCommand`]s, and loop until the
//! highlighted problem is resolved.

use nova_ecs::transform::{GlobalTransform, Transform};
use nova_ecs::{Entity, Mat4, Vec3, World};
use serde::{Deserialize, Serialize};

/// A rectangle in pixel coordinates (top-left origin, y-down).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenRect {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

impl ScreenRect {
    /// Construct, normalizing so (x0,y0) is the top-left and (x1,y1) the
    /// bottom-right regardless of which corner was dragged first.
    pub fn new(a: (u32, u32), b: (u32, u32)) -> Self {
        ScreenRect {
            x0: a.0.min(b.0),
            y0: a.1.min(b.1),
            x1: a.0.max(b.0),
            y1: a.1.max(b.1),
        }
    }

    pub fn width(&self) -> u32 {
        self.x1 - self.x0
    }
    pub fn height(&self) -> u32 {
        self.y1 - self.y0
    }

    /// Normalize to `[0,1]` coordinates given the viewport `size` (w, h).
    /// Returns in (x, y, w, h) form with y measured from the top.
    pub fn normalized(&self, size: (u32, u32)) -> [f32; 4] {
        let (w, h) = (size.0 as f32, size.1 as f32);
        [
            self.x0 as f32 / w,
            self.y0 as f32 / h,
            self.width() as f32 / w,
            self.height() as f32 / h,
        ]
    }

    pub fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }
}

/// Errors raised while building a fix request.
#[derive(Debug, thiserror::Error)]
pub enum OverlayError {
    #[error("no camera found in world to project entities")]
    NoCamera,
    #[error("region is empty (zero area)")]
    EmptyRegion,
}

/// Project a world-space point to viewport pixel coordinates using a precomputed
/// camera view-projection matrix and the viewport `size` (w, h).
///
/// Returns `None` for points behind the camera (clip `w <= 0`).
pub fn project_to_screen(
    world_point: Vec3,
    view_proj: Mat4,
    size: (u32, u32),
) -> Option<(u32, u32)> {
    let clip = view_proj * Vec4::new(world_point.x, world_point.y, world_point.z, 1.0);
    if clip.w <= 1e-6 {
        return None;
    }
    let ndc_x = clip.x / clip.w;
    let ndc_y = clip.y / clip.w;
    // NDC is [-1,1] with y up; screen is pixels with y down.
    let sx = ((ndc_x * 0.5 + 0.5) * size.0 as f32).round() as i64;
    let sy = ((1.0 - (ndc_y * 0.5 + 0.5)) * size.1 as f32).round() as i64;
    if sx < 0 || sy < 0 || sx > size.0 as i64 || sy > size.1 as i64 {
        return None;
    }
    Some((sx as u32, sy as u32))
}

/// Return the entities whose world transform center projects inside `region`.
///
/// `view_proj` is the camera's view-projection matrix (as `nova-render`
/// computes it). Entities without a transform are skipped.
pub fn pick_entities_in_region(
    world: &World,
    view_proj: Mat4,
    size: (u32, u32),
    region: ScreenRect,
) -> Vec<Entity> {
    let mut hits = Vec::new();
    for (e, _local, gt) in world.query_2::<Transform, GlobalTransform>() {
        // Prefer the world-space transform for stable picking.
        let center = gt.translation();
        if let Some((sx, sy)) = project_to_screen(center, view_proj, size) {
            if region.contains(sx, sy) {
                hits.push(e);
            }
        } else {
            // Fall back to the local transform's translation if the global one
            // projected behind the camera.
            let local = world.get_component::<Transform>(e).map(|l| l.translation);
            if let Some(p) = local {
                if let Some((sx, sy)) = project_to_screen(p, view_proj, size) {
                    if region.contains(sx, sy) {
                        hits.push(e);
                    }
                }
            }
        }
    }
    hits.sort_by_key(|e| e.index);
    hits
}

/// A structured request handed to an AI coding agent: where the problem is,
/// which entities are implicated, and what to do about it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiFixRequest {
    /// Normalized region (x, y, w, h) in `[0,1]`.
    pub region: [f32; 4],
    /// Entities whose centers fall inside the region, as telemetry id strings.
    pub entity_ids: Vec<String>,
    /// The natural-language instruction describing the desired fix.
    pub instruction: String,
    /// A ready-to-send prompt string combining the above.
    pub prompt: String,
}

impl AiFixRequest {
    /// Render the structured fields into a single prompt string.
    pub fn build_prompt(region: [f32; 4], entity_ids: &[String], instruction: &str) -> String {
        let ids = if entity_ids.is_empty() {
            "(no entities in region)".to_string()
        } else {
            entity_ids.join(", ")
        };
        format!(
            "FIX REQUEST\n\
             Region (normalized x,y,w,h): [{:.3}, {:.3}, {:.3}, {:.3}]\n\
             Entities in region: {}\n\
             Instruction: {}\n\
             Use the engine control API to resolve this.",
            region[0], region[1], region[2], region[3], ids, instruction
        )
    }
}

/// Build a fix request for `region` in a world viewed through `view_proj`.
///
/// This is the full "Highlight & Fix" entry point: pick the entities under the
/// highlight rectangle and assemble the agent prompt.
pub fn build_fix_request(
    world: &World,
    view_proj: Mat4,
    size: (u32, u32),
    region: ScreenRect,
    instruction: &str,
) -> Result<AiFixRequest, OverlayError> {
    if region.width() == 0 || region.height() == 0 {
        return Err(OverlayError::EmptyRegion);
    }
    let hits = pick_entities_in_region(world, view_proj, size, region);
    let entity_ids: Vec<String> = hits.iter().map(|e| format!("{e}")).collect();
    let region_n = region.normalized(size);
    let prompt = AiFixRequest::build_prompt(region_n, &entity_ids, instruction);
    Ok(AiFixRequest {
        region: region_n,
        entity_ids,
        instruction: instruction.to_string(),
        prompt,
    })
}

/// Bring `Vec4::from_point` into scope without a direct glam dependency leak.
use glam::Vec4;

/// Interactive "Highlight & Fix" selection tool.
///
/// The host feeds raw pointer events (press / move / release) in viewport pixel
/// coordinates; the tool tracks the drag marquee and, on release, assembles an
/// [`AiFixRequest`] for the highlighted region. It owns no GPU state and is fully
/// unit-testable — the renderer just needs [`SelectionTool::current_rect`] to
/// draw the live rectangle each frame.
#[derive(Debug, Clone, Default)]
pub struct SelectionTool {
    dragging: bool,
    start: Option<(u32, u32)>,
    current: Option<(u32, u32)>,
    last_request: Option<AiFixRequest>,
}

impl SelectionTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// True while a marquee drag is in progress.
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    /// Pointer press: begin a new selection at `(x, y)` (viewport pixels).
    pub fn begin(&mut self, x: u32, y: u32) {
        self.dragging = true;
        self.start = Some((x, y));
        self.current = Some((x, y));
    }

    /// Pointer move: update the far corner of the marquee while dragging.
    pub fn drag(&mut self, x: u32, y: u32) {
        if self.dragging {
            self.current = Some((x, y));
        }
    }

    /// Pointer release: finish the drag and return the completed marquee rect
    /// (normalized so the start corner is top-left), or `None` if the drag was
    /// not valid (no start, or zero area is handled by the caller).
    pub fn end(&mut self) -> Option<ScreenRect> {
        if !self.dragging {
            return None;
        }
        self.dragging = false;
        let (a, b) = match (self.start, self.current) {
            (Some(a), Some(b)) => (a, b),
            _ => return None,
        };
        if a == b {
            return None;
        }
        Some(ScreenRect::new(a, b))
    }

    /// The live marquee rectangle for the current drag, in viewport pixels.
    /// Returns `None` when not dragging or before the first move.
    pub fn current_rect(&self, _size: (u32, u32)) -> Option<ScreenRect> {
        match (self.start, self.current) {
            (Some(a), Some(b)) if self.dragging => Some(ScreenRect::new(a, b)),
            _ => None,
        }
    }

    /// The most recently built fix request (kept so the UI can show what was
    /// last sent even after the marquee is cleared).
    pub fn last_request(&self) -> Option<&AiFixRequest> {
        self.last_request.as_ref()
    }

    /// Finish the current drag (if any) and build the agent fix request for the
    /// highlighted region. Equivalent to [`end`] followed by
    /// [`build_fix_request`]; the resulting request is cached via
    /// [`last_request`].
    pub fn build_request(
        &mut self,
        world: &World,
        view_proj: Mat4,
        size: (u32, u32),
        instruction: &str,
    ) -> Result<AiFixRequest, OverlayError> {
        let region = self
            .end()
            .ok_or(OverlayError::EmptyRegion)?;
        let req = build_fix_request(world, view_proj, size, region, instruction)?;
        self.last_request = Some(req.clone());
        Ok(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_ecs::transform::{Camera, GlobalTransform, Mesh, MeshKind};
    use nova_ecs::World;

    fn camera_view_proj() -> (World, Mat4, (u32, u32)) {
        let mut world = World::new();
        let cam = world.spawn();
        // Camera at (0,0,5) looking down -Z; identity rotation.
        world.add_component(cam, Transform::from_translation(Vec3::new(0.0, 0.0, 5.0)));
        world.add_component(cam, Camera::default());
        world.add_component(cam, GlobalTransform::identity());
        // Run propagation so GlobalTransform reflects the local transform.
        nova_ecs::scene_graph::propagate_transforms(&mut world);

        // Compute view_proj manually: perspective * view(inverse of cam world).
        let cam_t = Transform::from_translation(Vec3::new(0.0, 0.0, 5.0));
        let view = cam_t.matrix().inverse();
        let proj = Camera {
            aspect: 1.0,
            ..Default::default()
        };
        let vp = proj.perspective() * view;
        (world, vp, (800, 600))
    }

    #[test]
    fn screen_rect_normalizes_and_contains() {
        let r = ScreenRect::new((100, 100), (300, 200));
        assert_eq!(r.x0, 100);
        assert_eq!(r.y0, 100);
        assert_eq!(r.x1, 300);
        assert_eq!(r.width(), 200);
        assert_eq!(
            r.normalized((800, 600)),
            [100.0 / 800.0, 100.0 / 600.0, 200.0 / 800.0, 100.0 / 600.0]
        );
        assert!(r.contains(150, 150));
        assert!(!r.contains(50, 50));
    }

    #[test]
    fn screen_rect_normalizes_drag_direction() {
        let r = ScreenRect::new((300, 200), (100, 100));
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (100, 100, 300, 200));
    }

    #[test]
    fn project_centers_visible_point_on_screen() {
        // World origin is 5 units in front of a camera at z=5 -> projects near
        // screen center (400, 300) for an 800x600 viewport.
        let (_, vp, size) = camera_view_proj();
        let p = project_to_screen(Vec3::ZERO, vp, size).unwrap();
        assert!((p.0 as i32 - 400).abs() < 5, "x near center, got {p:?}");
        assert!((p.1 as i32 - 300).abs() < 5, "y near center, got {p:?}");
    }

    #[test]
    fn picks_entity_inside_region() {
        let (mut world, vp, size) = camera_view_proj();
        // An entity at the world origin projects to screen center.
        let e = world.spawn();
        world.add_component(e, Transform::from_translation(Vec3::ZERO));
        world.add_component(
            e,
            Mesh {
                kind: MeshKind::Cube,
            },
        );
        world.add_component(e, GlobalTransform::identity());

        let region = ScreenRect::new((350, 250), (450, 350)); // covers center
        let hits = pick_entities_in_region(&world, vp, size, region);
        assert_eq!(hits, vec![e]);
    }

    #[test]
    fn build_fix_request_assembles_prompt_and_ids() {
        let (mut world, vp, size) = camera_view_proj();
        let e = world.spawn();
        world.add_component(e, Transform::from_translation(Vec3::ZERO));
        world.add_component(e, GlobalTransform::identity());
        let region = ScreenRect::new((350, 250), (450, 350));
        let req = build_fix_request(&world, vp, size, region, "make it red").unwrap();
        assert_eq!(req.entity_ids, vec![format!("{e}")]);
        assert!(req.prompt.contains("FIX REQUEST"));
        assert!(req.prompt.contains("make it red"));
        assert!(req.prompt.contains(&format!("{e}")));
    }

    #[test]
    fn empty_region_is_rejected() {
        let (world, vp, size) = camera_view_proj();
        let region = ScreenRect::new((10, 10), (10, 10));
        assert!(matches!(
            build_fix_request(&world, vp, size, region, "x"),
            Err(OverlayError::EmptyRegion)
        ));
    }

    #[test]
    fn region_away_from_entity_picks_nothing() {
        let (mut world, vp, size) = camera_view_proj();
        let e = world.spawn();
        // Far to the side -> projects off the small center region.
        world.add_component(e, Transform::from_translation(Vec3::new(50.0, 0.0, 0.0)));
        world.add_component(e, GlobalTransform::identity());
        // Propagate so the global transform reflects the local placement.
        nova_ecs::scene_graph::propagate_transforms(&mut world);
        let region = ScreenRect::new((350, 250), (450, 350));
        let hits = pick_entities_in_region(&world, vp, size, region);
        assert!(hits.is_empty());
    }

    #[test]
    fn selection_tool_tracks_marquee_and_builds_request() {
        let (mut world, vp, size) = camera_view_proj();
        let e = world.spawn();
        world.add_component(e, Transform::from_translation(Vec3::ZERO));
        world.add_component(e, GlobalTransform::identity());

        let mut tool = SelectionTool::new();
        assert!(!tool.is_dragging());
        tool.begin(360, 260);
        assert!(tool.is_dragging());
        // While dragging, the live rect tracks start + current.
        tool.drag(440, 340);
        let live = tool.current_rect(size).expect("live rect during drag");
        assert_eq!((live.x0, live.y0, live.x1, live.y1), (360, 260, 440, 340));

        // Release builds a fix request for the highlighted region.
        let req = tool
            .build_request(&world, vp, size, "make it red")
            .unwrap();
        assert_eq!(req.entity_ids, vec![format!("{e}")]);
        assert!(req.prompt.contains("make it red"));
        assert!(!tool.is_dragging());
        assert!(tool.last_request().is_some());
    }

    #[test]
    fn selection_tool_rejects_empty_drag() {
        let (world, vp, size) = camera_view_proj();
        let mut tool = SelectionTool::new();
        tool.begin(100, 100);
        tool.drag(100, 100); // no movement -> zero-area on release
        assert!(matches!(
            tool.build_request(&world, vp, size, "x"),
            Err(OverlayError::EmptyRegion)
        ));
    }
}
