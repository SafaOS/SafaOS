use std::process::Command;

use libgem::{
    Gem, GemConfig,
    element::button::{Button, ButtonStyle},
};

struct HelloWorld {
    count: usize,
}

impl Gem for HelloWorld {}
fn main() {
    let mut gem = HelloWorld { count: 0 }.init(GemConfig::new("Hello, world!", 400, 400));

    let button_styles = ButtonStyle::new(40, 40);
    let mut clicker_button = Button::new("Click Me", button_styles);
    clicker_button.on_click(|btn, app: &mut HelloWorld| {
        let amount = app.count;
        app.count += 1;
        btn.set_label(&format!("{} Click(s)", amount));
    });

    let mut spawn_process_button = Button::new("Spawn Process", button_styles);
    spawn_process_button.on_click(|_, _| {
        Command::new("sys:/bin/hello_world")
            .spawn()
            .expect("Failed to spawn hello world process");
    });

    let mut spawn_sysinfo_button = Button::new("Spawn Sysinfo", button_styles);
    spawn_sysinfo_button.on_click(|_, _| {
        Command::new("sys:/bin/sysver")
            .spawn()
            .expect("Failed to spawn sysinfo process");
    });

    let mut spawn_calc_button = Button::new("Spawn Calc", button_styles);
    spawn_calc_button.on_click(|_, _| {
        Command::new("sys:/bin/calc")
            .spawn()
            .expect("Failed to spawn calc process");
    });

    let mut spawn_terminal_button = Button::new("Spawn Terminal", button_styles);
    spawn_terminal_button.on_click(|_, _| {
        println!("Spawning terminal with vars");
        for (name, value) in std::env::vars_os() {
            println!("{name:?}={value:?}")
        }

        Command::new("sys:/bin/terminal")
            .spawn()
            .expect("Failed to spawn terminal process");
    });

    gem.add_element(clicker_button);
    gem.add_element(spawn_process_button);
    gem.add_element(spawn_sysinfo_button);
    gem.add_element(spawn_calc_button);
    gem.add_element(spawn_terminal_button);

    loop {
        gem.redraw();
        let events = gem.handle_events_blocking();
    }
}
