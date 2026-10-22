use crate::{logging, task::try_spawn as spawn};
use libkernel::rt::DynFut;

pub async fn eve_main() {
    spawn(async {
        DynFut::new(async move { logging::info!("eve", "Hello 2!") }).await;
    })
    .unwrap();
    spawn(async {
        let f = DynFut::new(async move { logging::info!("eve", "Hello 3!") });
        let s = DynFut::new(async move { logging::info!("eve", "Hello 4!") });

        s.await;
        f.await;
    })
    .unwrap();

    logging::info!("eve", "Hello!");
}
