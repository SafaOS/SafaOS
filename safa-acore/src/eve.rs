use crate::{logging, task::spawn};

pub async fn eve_main() {
    spawn(async move { logging::info!("eve", "Hello 2!") });
    spawn(async move { logging::info!("eve", "Hello 3!") });
    spawn(async move { logging::info!("eve", "Hello 4!") });

    logging::info!("eve", "Hello!");
}
