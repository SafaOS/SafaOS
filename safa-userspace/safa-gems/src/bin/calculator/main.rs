use libgem::cosmic_text::Metrics;
use libgems::{
    App, AppState, Color, Data, Padding, WindowBuilder,
    shards::{AxisAlign, Button, Justify, Label, Shard, ShardsExt, Stack},
};

use crate::logic::LexerData;
mod logic;

const WIDTH: u32 = 230;
const HEIGHT: u32 = 320;
use logic::Operation as Message;

pub struct Calculator {
    logic: LexerData,
    current: String,
    value: Result<f64, &'static str>,
}

impl AppState for Calculator {
    type Message = Message;
}

fn buttons_area() -> impl Shard<Data<Calculator>> {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Size {
        Normal,
        Big,
    }

    macro_rules! btn {
        ($action:ident, $name:literal $(, $size:ident)?) => {{
            #[allow(unused_assignments, unused_mut)]
            let mut size = Size::Normal;
            $(size = Size::$size;)?
            ($name, Message::$action, size)
        }};
        ($num:literal) => {
            (stringify!($num), Message::Number($num), Size::Normal)
        };
        ($num:literal, $name:literal) => {
            ($name, Message::Number($num), Size::Normal)
        };
    }

    const BUTTONS: [(&'static str, Message, Size); 21] = [
        btn!(Clear, "AC", Big),
        btn!(Remove, "⌫", Big),
        btn!(1, "("),
        btn!(1, ")"),
        btn!(Div, "%"),
        btn!(Div, "/"),
        btn!(1),
        btn!(2),
        btn!(3),
        btn!(Add, "+"),
        btn!(4),
        btn!(5),
        btn!(6),
        btn!(Sub, "-"),
        btn!(7),
        btn!(8),
        btn!(9),
        btn!(Mul, "*"),
        btn!(0),
        btn!(Dot, "."),
        btn!(Results, "=", Big),
    ];

    let mut btns = BUTTONS.iter();

    let mut current_stack: Stack<Data<Calculator>> = Stack::column();
    let mut left_in_current = 4;
    let mut stacks = Vec::new();

    let btn_pad = 4.;
    while let Some((name, action, size)) = btns.next() {
        left_in_current -= 1;
        if *size == Size::Big {
            left_in_current -= 1;
        }

        current_stack = current_stack.with_flex(
            Button::new(Label::from_str(*name).with_color(Color::hex_rgb(0xFBF1C7)))
                .with_paint(match action {
                    Message::Add | Message::Mul | Message::Sub | Message::Div => {
                        Color::hex_rgb(0x8b365c)
                    }
                    Message::Clear | Message::Results | Message::Remove => Color::hex_rgb(0xb33670),
                    // Message::Results => Color::hex_rgb(0xfe8019),
                    _ => Color::hex_rgb(0x783653),
                })
                .on_click(move |_, ctx: &mut Data<Calculator>, _| {
                    ctx.broadcast_message(*action);
                })
                .fix_height(32.)
                .pad(Padding::lr(btn_pad)),
            if *size == Size::Big { 2. } else { 1. },
        );

        if left_in_current == 0 {
            stacks.push(
                core::mem::replace(&mut current_stack, Stack::column())
                    .with_padding(Padding::none())
                    .justify(Justify::Center)
                    .pad(Padding::equal(4.)),
            );
            left_in_current = 4;
        }
    }

    let mut final_stack = Stack::row();
    for stack in stacks {
        final_stack = final_stack.with(stack);
    }

    final_stack
        .align(AxisAlign::Center)
        .with_padding(Padding::none())
        .fix_width(WIDTH as f32)
}
fn expr_screen() -> impl Shard<Data<Calculator>> {
    Stack::row()
        .with_padding(Padding::none())
        .with(
            Label::from_str("0000")
                .with_color(Color::hex_rgb(0xFBF1C7))
                .with_metrics(Metrics::relative(17., 1.0))
                .with_wrap(libgem::cosmic_text::Wrap::None)
                .fix_height(17.)
                .on_update(|ctx: &Data<Calculator>, this| {
                    if !ctx.current.is_empty() {
                        this.set_text(&ctx.current);
                    } else {
                        this.set_text("0000");
                    }
                })
                .pad(Padding::equal(4.)),
        )
        .with(
            Label::from_str("99230")
                .with_color(Color::hex_rgb(0xFBF1C7))
                .with_metrics(Metrics::relative(12., 1.0))
                .on_update(|ctx: &Data<Calculator>, this| {
                    match ctx.value {
                        Err(_) => this.set_text("Error"),
                        Ok(n) => this.set_text(&format!("{n}")),
                    };
                })
                .fix_height(12.)
                .pad(Padding::equal(8.))
                .align(AxisAlign::End),
        )
        .background(Color::hex_rgb(0xb33470))
        .round(12.)
        .fix_height(50.)
        .fix_width(218.)
        .pad(Padding::equal(8.))
}

fn build_ui() -> impl Shard<Data<Calculator>> {
    Stack::row()
        .align(AxisAlign::Center)
        .with(expr_screen())
        .with(buttons_area())
        .justify(Justify::SpaceBetween)
        .on_msg(|ctx: &mut Data<Calculator>, m, _| {
            if let Err(e) = ctx.logic.execute(&m) {
                ctx.current.clear();
                ctx.current.push_str(e);
            } else {
                let data = &mut **ctx;
                data.logic.view_to(&mut data.current);
            }

            ctx.value = ctx.logic.eval();
        })
}

fn main() {
    let mut app = App::new(Calculator {
        value: Ok(0.),
        current: String::new(),
        logic: LexerData::default(),
    })
    .window(
        WindowBuilder::new(WIDTH, HEIGHT)
            .title("Calculator")
            .background(Color::hex_rgb(0x3c3836))
            .build::<_, _>(build_ui()),
    );

    loop {
        if app.needs_redraw() {
            app.redraw_needed();
        }

        app.wait_for_events();
    }
}
