use libgems::{
    App, AppEnv, Padding, WindowBuilder,
    cosmic_text::{Attrs, Metrics, Weight},
    shards::{Image, Label, Shard, ShardsExt, Stack},
};

static LOGO: &[u8] = include_bytes!("../../assets/logo.qoi");
static ICON: &[u8] = include_bytes!("../../assets/logo.bmp");

const WINDOW_WIDTH: u32 = 220;
const WINDOW_HEIGHT: u32 = 280;

const DESCRIPTION: &str = {
    #[cfg(target_arch = "x86_64")]
    {
        "The SafaOS Operating System\nVersion v0.6.0 (x86_64)\nCopyright © 2026 SafaOS\nLicensed under the MIT License\nhttps://github.com/SafaOS/SafaOS\n\nMade with 🖤 and 🍓s by safiworks"
    }
    #[cfg(target_arch = "aarch64")]
    {
        "The SafaOS Operating System\nVersion v0.6.0 (aarch64)\nCopyright © 2026 SafaOS\nLicensed under the MIT License\nhttps://github.com/SafaOS/SafaOS\n\nMade with 🖤 and 🍓s by safiworks"
    }
};

struct SysVer;
fn build_ui() -> impl Shard<SysVer> {
    Stack::row()
        .justify(libgems::shards::Justify::SpaceBetween)
        .align(libgems::shards::AxisAlign::Center)
        .with(
            Image::from_image(
                image::load_from_memory(LOGO)
                    .expect("Failed to load logo")
                    .resize_exact(76, 76, image::imageops::FilterType::Triangle),
            )
            .pad(Padding::equal(20.)),
        )
        .with(
            Label::from_str("SafaOS")
                .with_metrics(Metrics::new(16., 16.))
                .with_attrs(Attrs::new().weight(Weight::SEMIBOLD)),
        )
        .with(
            Label::from_str(DESCRIPTION)
                .with_align(libgems::cosmic_text::Align::Center)
                .with_metrics(Metrics::new(12., 16.))
                .with_attrs(Attrs::new().weight(Weight::MEDIUM))
                .fix_height(120.),
        )
}
fn main_io() {
    let mut app = App::new(SysVer).with_env(AppEnv::sys_theme()).window(
        WindowBuilder::new(WINDOW_WIDTH, WINDOW_HEIGHT)
            .title("SysVer")
            .icon(ICON)
            .build(build_ui()),
    );

    loop {
        app.wait_for_events();
    }
}

fn main() {
    main_io()
}
