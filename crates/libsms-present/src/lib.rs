//! Presentation shared by every host that draws Mario itself: [`view`] (the disc's models posed
//! from the sim and CPU-skinned each frame), [`tev`] (each material's TEV baked into one
//! texture, for host shaders that take one texture times one colour) and [`audio`] (the game's
//! sounds for the sim's animation and events, on a cpal stream).

pub mod audio;
pub mod tev;
pub mod view;

pub use audio::Audio;
pub use view::View;
