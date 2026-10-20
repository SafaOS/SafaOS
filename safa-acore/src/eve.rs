use crate::{logging, task::try_spawn as spawn};

pub async fn eve_main() {
    spawn(async move { logging::info!("eve", "Hello 2!") }).unwrap();
    spawn(async move { logging::info!("eve", "Hello 3!") }).unwrap();
    spawn(async move { logging::info!("eve", "Hello 4!") }).unwrap();

    logging::info!("eve", "Hello!");
}
