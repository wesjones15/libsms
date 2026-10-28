//! Presentation shared by every host that draws Mario itself: [`view`] (the disc's models posed
//! from the sim and CPU-skinned each frame), [`tev`] (each material's TEV baked into one
//! texture, for host shaders that take one texture times one colour), [`water`] (FLUDD's droplets
//! and splashes as quads), [`hud`] (FLUDD's water gauge as one image), [`life`] (Mario's life meter, the sun) and [`audio`] (the game's sounds for the sim's animation and events, on
//! a cpal stream).

pub mod audio;
pub mod body;
pub mod coins;
pub mod democam;
pub mod hud;
pub mod life;
pub mod particles;
pub mod portal;
pub mod portal_cache;
pub mod plaza;
pub mod screenfx;
pub mod tev;
pub mod view;
pub mod water;

pub use audio::Audio;
pub use view::{Look, View};
pub use water::WaterLook;
