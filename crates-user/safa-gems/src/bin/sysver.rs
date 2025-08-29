use libgem::{
    Gem, GemConfig,
    element::{
        container::{ContainerLayout, ContainerStyles, VerticalLayout},
        image::{Image, ImageData},
        text_box::{TextBox, TextBoxStyles},
    },
    image::{QOIImage, ScaleType},
};

static LOGO: &[u8] = include_bytes!("../../assets/logo.qoi");
const WINDOW_WIDTH: u32 = 220;
const WINDOW_HEIGHT: u32 = 280;

const DESCRIPTION: &str = "The SafaOS Operating System\nVersion v0.4.0 (x86_64)\nCopyright © 2024 SafaOS\nLicensed under the MIT License\nhttps://github.com/SafaOS/SafaOS\n\nMade with ❤️ by safiworks";
const DESCRIPTION_STYLES: TextBoxStyles =
    TextBoxStyles::new(WINDOW_WIDTH as f32, WINDOW_HEIGHT as f32);

const HEADER_STYLES: TextBoxStyles = TextBoxStyles::new(80., 25.)
    .with_font_size(16.0)
    .with_line_padding(0.);
struct SysVer;

impl Gem for SysVer {}
const BODY_STYLES: ContainerStyles = ContainerStyles::new().with_layout(ContainerLayout::Vertical(
    VerticalLayout::new().with_align_center(true),
));

fn main_io() {
    let app_styles =
        GemConfig::new("SysVer", WINDOW_WIDTH, WINDOW_HEIGHT).with_body_styles(BODY_STYLES);
    let mut app = SysVer.init(app_styles);

    let header = TextBox::new("SafaOS", HEADER_STYLES);
    let logo_qoi = QOIImage::decode(LOGO).expect("Failed to parse logo");
    let scaled_logo = logo_qoi.into_scaled_image(128, 128, ScaleType::Triangle);

    let logo_image = Image::new(ImageData::Generic(scaled_logo));
    let description = TextBox::new(DESCRIPTION, DESCRIPTION_STYLES);

    app.add_element(logo_image);
    app.add_element(header);
    app.add_element(description);

    loop {
        app.redraw();
        app.handle_events_blocking();
    }
}

fn main() {
    main_io()
}
