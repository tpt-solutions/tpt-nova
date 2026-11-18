//! The streamed-frame contract: a [`FrameSource`] yields decoded frames.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::frame::Frame;

/// A source of decoded video frames for one neural material.
///
/// Returns `None` when the stream has ended (or is temporarily starved); the
/// engine treats a steady `None` as "hold the last frame".
pub trait FrameSource: Send + Sync {
    /// Block briefly (or not at all) for the next available frame.
    fn next_frame(&mut self) -> Option<Frame>;
}

/// A single static image reused as a looping texture.
///
/// Useful for demos and for materials whose prompt resolves to one image
/// (e.g. an AI-generated poster) rather than a moving clip.
pub struct StaticImageSource {
    frame: Frame,
    looping: bool,
    consumed: bool,
}

impl StaticImageSource {
    pub fn new(frame: Frame, looping: bool) -> Self {
        StaticImageSource {
            frame,
            looping,
            consumed: false,
        }
    }
}

impl FrameSource for StaticImageSource {
    fn next_frame(&mut self) -> Option<Frame> {
        if self.looping {
            return Some(self.frame.clone());
        }
        if self.consumed {
            None
        } else {
            self.consumed = true;
            Some(self.frame.clone())
        }
    }
}

/// A producer/consumer frame queue.
///
/// Lets an external process (or a test) inject decoded frames without owning a
/// decoder. The engine polls [`next_frame`](FrameSource::next_frame); the
/// generative side calls [`push`](PushingSource::push).
pub struct PushingSource {
    queue: Arc<Mutex<VecDeque<Frame>>>,
}

impl PushingSource {
    pub fn new() -> Self {
        PushingSource {
            queue: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Push a decoded frame onto the queue (called by the feed side).
    pub fn push(&self, frame: Frame) {
        if let Ok(mut q) = self.queue.lock() {
            q.push_back(frame);
        }
    }

    /// Number of frames currently buffered.
    pub fn pending(&self) -> usize {
        self.queue.lock().map(|q| q.len()).unwrap_or(0)
    }
}

impl Default for PushingSource {
    fn default() -> Self {
        PushingSource::new()
    }
}

impl FrameSource for PushingSource {
    fn next_frame(&mut self) -> Option<Frame> {
        self.queue.lock().ok().and_then(|mut q| q.pop_front())
    }
}
