use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Sub, SubAssign};

use libgem::{
    Gem, GemConfig,
    element::{
        button::{Button, ButtonStyle},
        container::{ContainerLayout, ContainerStyles, GridLayout},
        text_box::{TextBox, TextBoxStyles},
    },
};

const WIDTH: u32 = 240;
const HEIGHT: u32 = 200;
const TITLE: &str = "Calculator";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Number {
    Int(isize),
    Err(&'static str),
}

impl Number {
    pub fn and_then<F>(self, f: F) -> Number
    where
        F: FnOnce(isize) -> Number,
    {
        match self {
            Number::Int(num) => f(num),
            Number::Err(err) => Number::Err(err),
        }
    }

    pub const fn is_err(&self) -> bool {
        match self {
            Number::Int(_) => false,
            Number::Err(_) => true,
        }
    }

    pub fn and_then_2<F>(self, other: Self, f: F) -> Number
    where
        F: FnOnce(isize, isize) -> Number,
    {
        match (self, other) {
            (Number::Int(num1), Number::Int(num2)) => f(num1, num2),
            (Number::Err(err), _) | (_, Number::Err(err)) => Number::Err(err),
        }
    }
}

macro_rules! impl_trait {
    ($name: ty, $name_int: ty, $lowercase: ident, $func_name: ident, $assign_name: ident, $assign_name_lower: ident) => {
        impl $name for Number {
            type Output = Number;
            fn $lowercase(self, rhs: Number) -> Self::Output {
                self.and_then_2(rhs, |s, o| {
                    isize::$func_name(s, o)
                        .map(|i| Number::Int(i))
                        .unwrap_or(Number::Err("Overflow"))
                })
            }
        }

        impl $name_int for Number {
            type Output = Number;
            fn $lowercase(self, rhs: isize) -> Self::Output {
                self.and_then(|s| {
                    isize::$func_name(s, rhs)
                        .map(|i| Number::Int(i))
                        .unwrap_or(Number::Err("Overflow"))
                })
            }
        }

        impl $assign_name for Number {
            fn $assign_name_lower(&mut self, rhs: Number) {
                *self = self.$lowercase(rhs);
            }
        }
    };
}

impl_trait!(
    Add<Number>,
    Add<isize>,
    add,
    checked_add,
    AddAssign,
    add_assign
);
impl_trait!(
    Sub<Number>,
    Sub<isize>,
    sub,
    checked_sub,
    SubAssign,
    sub_assign
);
impl_trait!(
    Mul<Number>,
    Mul<isize>,
    mul,
    checked_mul,
    MulAssign,
    mul_assign
);

impl Div<Number> for Number {
    type Output = Number;
    fn div(self, rhs: Number) -> Self::Output {
        self.and_then_2(rhs, |s, o| {
            s.checked_div(o)
                .map(|i| Number::Int(i))
                .unwrap_or(if o == 0 {
                    Number::Err("Divide by Zero")
                } else {
                    Number::Err("Overflow")
                })
        })
    }
}

impl DivAssign<Number> for Number {
    fn div_assign(&mut self, rhs: Number) {
        *self = self.div(rhs);
    }
}

enum CurrentState {
    Idle,
    Adding(Number),
    Subtracting(Number),
    Multiplying(Number),
    Dividing(Number),
}
pub struct Calc {
    current_number: Number,
    current_state: CurrentState,
}

impl Calc {
    pub const fn new() -> Self {
        Self {
            current_number: Number::Int(0),
            current_state: CurrentState::Idle,
        }
    }

    fn apply_operation(&mut self) {
        match self.current_state {
            CurrentState::Idle => (),
            CurrentState::Adding(value) => {
                self.current_number = value + self.current_number;
            }
            CurrentState::Subtracting(value) => {
                self.current_number = value - self.current_number;
            }
            CurrentState::Multiplying(value) => {
                self.current_number = value * self.current_number;
            }
            CurrentState::Dividing(value) => {
                self.current_number = value / self.current_number;
            }
        }
    }
}

impl Gem for Calc {}

fn main() {
    let config =
        GemConfig::new(TITLE, WIDTH, HEIGHT).with_body_styles(ContainerStyles::new().with_layout(
            ContainerLayout::Grid(GridLayout::new().with_elements_per_row(4)),
        ));
    let mut app = Calc::new().init(config);

    let numbers_box_styles = TextBoxStyles::new(230.0, 20.0);
    let numbers_box = TextBox::new("0", numbers_box_styles);
    let label_id = app.add_element(numbers_box);

    let numbers_buttons_styles = ButtonStyle::new(WIDTH / 5, 30);

    macro_rules! add_num_button {
        ($num:literal) => {{
            let mut button = Button::new(stringify!($num), numbers_buttons_styles);
            button.on_click(move |_, gem: &mut Calc| {
                // Add the digit num to the current number
                if gem.current_number.is_err() {
                    gem.current_number = Number::Int($num);
                } else {
                    gem.current_number = gem.current_number * 10 + $num;
                }
            });
            app.add_element(button);
        }};
    }

    let mut backspace_button = Button::new("⌫", numbers_buttons_styles);
    backspace_button.on_click(move |_, gem: &mut Calc| {
        if let Number::Int(num) = gem.current_number {
            gem.current_number = Number::Int(num / 10);
        } else {
            gem.current_number = Number::Int(0);
        }
    });

    let mut add_button = Button::new("+", numbers_buttons_styles);
    add_button.on_click(move |_, gem: &mut Calc| {
        gem.apply_operation();
        gem.current_state = CurrentState::Adding(gem.current_number);
        gem.current_number = Number::Int(0);
    });

    let mut subtract_button = Button::new("-", numbers_buttons_styles);
    subtract_button.on_click(move |_, gem: &mut Calc| {
        gem.apply_operation();
        gem.current_state = CurrentState::Subtracting(gem.current_number);
        gem.current_number = Number::Int(0);
    });

    let mut multiply_button = Button::new("*", numbers_buttons_styles);
    multiply_button.on_click(move |_, gem: &mut Calc| {
        gem.apply_operation();
        gem.current_state = CurrentState::Multiplying(gem.current_number);
        gem.current_number = Number::Int(0);
    });

    let mut divide_button = Button::new("/", numbers_buttons_styles);
    divide_button.on_click(move |_, gem: &mut Calc| {
        gem.apply_operation();
        gem.current_state = CurrentState::Dividing(gem.current_number);
        gem.current_number = Number::Int(0);
    });

    let mut equals_button = Button::new("=", numbers_buttons_styles);
    equals_button.on_click(move |_, gem: &mut Calc| {
        gem.apply_operation();
        gem.current_state = CurrentState::Idle;
    });

    add_num_button!(7);
    add_num_button!(8);
    add_num_button!(9);
    app.add_element(subtract_button);

    add_num_button!(4);
    add_num_button!(5);
    add_num_button!(6);
    app.add_element(add_button);

    add_num_button!(1);
    add_num_button!(2);
    add_num_button!(3);
    app.add_element(multiply_button);

    app.add_element(divide_button);
    add_num_button!(0);

    app.add_element(backspace_button);
    app.add_element(equals_button);

    loop {
        app.redraw();

        let last_number = app.gem().current_number;
        app.handle_events_blocking();
        let curr_num = app.gem().current_number;

        if last_number != curr_num {
            let text_box = app.body().get_element_as_mut::<TextBox>(label_id).unwrap();
            match curr_num {
                Number::Int(num) => {
                    println!("Number {num}");
                    text_box.set_text(&format!("{num}"));
                }
                Number::Err(err) => text_box.set_text(err),
            }
        }
    }
}
