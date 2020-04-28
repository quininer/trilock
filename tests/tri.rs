#[cfg(not(feature = "loom"))]
use std::thread;

#[cfg(feature = "loom")]
use loom::thread;

use trilock::TriLock;


fn test_trilock() {
    let (one, two, three) = TriLock::new(0u32);

    let j = thread::spawn(move || futures_executor::block_on(async {
        let mut lock = one.lock().await;
        *lock += 1;
    }));

    let j2 = thread::spawn(move || futures_executor::block_on(async {
        let mut lock = two.lock().await;
        *lock += 2;
    }));

    let _ = j.join();
    let _ = j2.join();

    futures_executor::block_on(async {
        let lock = three.lock().await;
        assert_eq!(3, *lock);
    });
}

#[cfg(not(feature = "loom"))]
#[test]
fn test_normal_trilock() {
    test_trilock();
}
