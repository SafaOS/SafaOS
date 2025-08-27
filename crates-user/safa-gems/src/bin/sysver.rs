use libgem::{
    BORDER_COLOR0, DARK_BG_COLOR1, Gem, LIGHT_BG_COLOR0,
    element::{ContainerLayout, Image, ImageData, Label},
    image::{QOIImage, ScaleType},
    libopal::window::Pixel,
};

static LOGO: &[u8] = include_bytes!("../../assets/logo.qoi");
const BLACK: Pixel = Pixel::from_rgb(0, 0, 0);

fn main_io() {
    const WINDOW_WIDTH: u32 = 240;
    const WINDOW_HEIGHT: u32 = 240;
    let mut app = Gem::init(
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        "SysVer",
        LIGHT_BG_COLOR0,
        BORDER_COLOR0,
    );
    app.set_layout(ContainerLayout::Vertical { align_center: true });

    let mut header = Label::new("SafaOS", 16.0, 16.0, 80.0, 20.0);
    let logo_qoi = QOIImage::decode(LOGO).expect("Failed to parse logo");
    let scaled_logo = logo_qoi.into_scaled_image(128, 128, ScaleType::Triangle);

    let logo_image = Image::new(ImageData::PixelImage(scaled_logo));
    let mut descriptor = Label::new(
        "The SafaOS Operating System\nVersion v0.4.0 (x86_64)\nCopyright © 2024 SafaOS\nLicensed under the MIT License\nhttps://github.com/SafaOS/SafaOS\n\nMade with ❤️ by safiworks",
        10.0,
        12.0,
        WINDOW_WIDTH as f32,
        WINDOW_HEIGHT as f32,
    );

    descriptor.set_color(BLACK);
    header.set_color(BLACK);

    app.add_element(Box::new(logo_image));
    app.add_element(Box::new(header));
    app.add_element(Box::new(descriptor));

    loop {
        app.redraw();
        app.handle_events_blocking();
    }
}

fn main() {
    main_io()
}
