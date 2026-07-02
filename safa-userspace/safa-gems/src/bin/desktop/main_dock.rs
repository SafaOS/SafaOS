use libgem::image::{BMPImage, PixelImage};
use libgems::{
    AppEnv, Data, Padding,
    shards::{Shard, ShardsExt, Stack},
    theme,
};
use libopal::shm::SharedObject;

use crate::task_button::TaskButton;

static FALLBACK_ICON_BMP: &[u8] = include_bytes!("../../../assets/unknown.bmp");

fn fallback_icon() -> PixelImage {
    BMPImage::from_slice(FALLBACK_ICON_BMP)
        .expect("Failed to parse fallback image")
        .into()
}

#[derive(Debug)]
pub struct DockData {
    icon_cache: SharedObject,
}

impl DockData {
    pub fn new() -> Self {
        Self {
            icon_cache: SharedObject::allocate(256 * 256 * 4)
                .expect("Failed to allocate space to cache icons"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockMessage {
    Attached(u16),
    Deatached(u16),
    FocusChanged(u16, bool),
}

fn build_ui_inner(env: &AppEnv) -> impl Shard<DockData, DockMessage> + 'static {
    Stack::column()
        .justify(libgems::shards::Justify::Center)
        .on_msg(
            |_, data: &mut Data<DockData, DockMessage>, m, this| match m {
                DockMessage::Attached(w) => {
                    let Ok(info) = libopal::window::window_info(*w) else {
                        return;
                    };

                    let icon = (|| {
                        if let Some(icon) = info.icon_id() {
                            let Ok(raw_data) = libopal::icon::load_icon(&mut data.icon_cache, icon)
                            else {
                                return fallback_icon();
                            };
                            let bmp = match BMPImage::from_slice(&raw_data) {
                                Ok(k) => k,
                                Err(e) => {
                                    println!(
                                        "Error prasing BMP Icon: {e} for window: {}",
                                        info.name()
                                    );
                                    return fallback_icon();
                                }
                            };

                            bmp.into()
                        } else {
                            fallback_icon()
                        }
                    })();

                    *this = core::mem::replace(this, Stack::column()).with_flex(
                        TaskButton::new(*w, icon)
                            .on_msg(
                                |ctx, _: &mut Data<DockData, DockMessage>, m, this| match m {
                                    DockMessage::Deatached(w) if this.win_id == *w => {
                                        ctx.request_remove();
                                    }
                                    DockMessage::FocusChanged(w, focus) if this.win_id == *w => {
                                        this.set_focused(*focus);
                                    }
                                    _ => {}
                                },
                            )
                            .on_click(|_, _, btn| {
                                _ = libopal::window::focus_window(btn.win_id);
                            }),
                        1.,
                    );
                }
                _ => {}
            },
        )
        .size_pad(Padding::equal(4.))
        .background(env.get(theme::BACKGROUND_COLOR))
        .round(12.)
}

pub fn build_ui(
    env: &AppEnv,
    width: u32,
    height: u32,
) -> impl Shard<DockData, DockMessage> + 'static {
    Stack::column()
        .with(build_ui_inner(env))
        .justify(libgems::shards::Justify::SpaceAround)
        .fix_size(width as f32, height as f32)
}
