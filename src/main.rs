extern crate clouseau;
extern crate crossbeam;
extern crate crypto as rust_crypto;
extern crate fern;
extern crate futures;
extern crate futures_cpupool;
extern crate gcrypt;
extern crate hyper;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
extern crate nanomsg;
#[macro_use]
extern crate quick_error;
extern crate rusqlite;
extern crate rustc_serialize as serialize;
extern crate serde;
extern crate serde_json;
extern crate serde_yaml;
extern crate time;

#[macro_use]
mod error;
mod config;
#[macro_use]
mod util;
mod messaging;
mod storage;
mod api;
mod crypto;
mod models;
mod dispatch;
mod turtl;

use ::std::thread;
use ::std::sync::Arc;

use ::crossbeam::sync::MsQueue;

use ::error::{TError, TResult};
use ::util::event::Emitter;
use ::util::stopper::Stopper;
use ::util::thredder::Pipeline;

/// Init any state/logging/etc the app needs
pub fn init() -> TResult<()> {
    match util::logger::setup_logger() {
        Ok(..) => Ok(()),
        Err(e) => Err(toterr!(e)),
    }
}

lazy_static!{
    static ref RUN: Stopper = Stopper::new();
}

/// Stop all threads and close down Turtl
pub fn stop(tx: Pipeline) {
    (*RUN).set(false);
    tx.push(Box::new(move |_| {}));
}

/// Start our app...spawns all our worker/helper threads, including our comm
/// system that listens for external messages.
pub fn start(db_location: String) -> thread::JoinHandle<()> {
    (*RUN).set(true);
    thread::Builder::new().name(String::from("turtl-main")).spawn(move || {
        let queue_main = Arc::new(MsQueue::new());

        // start our messaging thread
        let (tx_msg, handle) = messaging::start(queue_main.clone());

        // create our turtl object
        let turtl = match turtl::Turtl::new_wrap(queue_main.clone(), tx_msg, &db_location) {
            Ok(x) => x,
            Err(err) => {
                error!("main::start() -- error creating Turtl object: {}", err);
                return;
            }
        };

        // bind turtl.events "app:shutdown" to close everything
        {
            let ref mut events = turtl.write().unwrap().events;
            let tx_main_shutdown = queue_main.clone();
            events.bind("app:shutdown", move |_| {
                stop(tx_main_shutdown.clone());
            }, "app:shutdown");
        }

        let api_endpoint: String = match config::get(&["api", "endpoint"]) {
            Ok(x) => x,
            Err(_) => String::from("https://api.turtlapp.com/v2"),
        };
        turtl.write().unwrap().api.set_endpoint(api_endpoint);

        // run our main loop. all threads pipe their data/responses into this
        // loop, meaning <main> only has to check one place to grab messages.
        // this creates an event loop of sorts, without all the grossness.
        while (*RUN).running() {
            debug!("turtl: main thread message loop");
            let handler = queue_main.pop();
            handler.call_box(turtl.clone());
        }
        info!("main::start() -- shutting down");
        turtl.write().unwrap().shutdown();
        match handle.join() {
            Ok(..) => {},
            Err(e) => error!("main: problem joining message thread: {:?}", e),
        }
    }).unwrap()
}

/// !!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
/// TODO: when calling this from C, handle all panics, or get rid of panics.
/// see https://doc.rust-lang.org/std/panic/fn.catch_unwind.html
/// !!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
fn main() {
    init().unwrap();
    start(String::from("d:/tmp/turtl-rs.sqlite")).join().unwrap();
}

