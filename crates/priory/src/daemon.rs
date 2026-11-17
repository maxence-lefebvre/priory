use crate::factory::Factory;
use crate::intake::OUTPUT;
use crate::state;
use herdr_bus::{Bus, Start};
use rusqlite::Connection;
use std::time::Duration;

const CONSUMER: &str = "priory";
// shortcut: polls the bus file; switch to herdr-bus-client's notify with the systemd service.
const EVERY: Duration = Duration::from_secs(1);

/// Handles every message on `factory.intake` past the daemon's cursor; without `once`, keeps waiting for more.
pub fn run(factory: &Factory, once: bool) -> Result<(), String> {
    let mut db = state::open()?;
    let bus = Bus::open_default().map_err(|error| error.to_string())?;
    loop {
        let handled = drain(&bus, &mut db, factory).map_err(|error| error.to_string())?;
        if once {
            return Ok(());
        }
        if handled == 0 {
            std::thread::sleep(EVERY);
        }
    }
}

fn drain(
    bus: &Bus,
    db: &mut Connection,
    factory: &Factory,
) -> Result<usize, Box<dyn std::error::Error>> {
    match bus.connect(CONSUMER, OUTPUT, Start::Beginning) {
        // The intake has not declared its output yet: nothing is pending.
        Err(herdr_bus::Error::UnknownOutput(_)) => return Ok(0),
        connected => connected?,
    };
    let mut handled = 0;
    loop {
        let deliveries = bus.pull(CONSUMER, 100)?;
        if deliveries.is_empty() {
            return Ok(handled);
        }
        for delivery in deliveries {
            state::handle(db, &delivery.message, &factory.workflows)?;
            bus.ack(delivery.connection, delivery.seq)?;
            handled += 1;
        }
    }
}
