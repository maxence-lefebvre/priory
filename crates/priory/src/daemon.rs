use crate::factory::Factory;
use crate::intake::OUTPUT;
use crate::runtime::{self, Launch, Line};
use crate::state::{self, Admission, NewRun, Signal};
use herdr_bus::{Bus, Start};
use rusqlite::Connection;
use std::path::Path;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

const CONSUMER: &str = "priory";
// shortcut: polls the bus file; switch to herdr-bus-client's notify with the systemd service.
const EVERY: Duration = Duration::from_secs(1);

type Error = Box<dyn std::error::Error>;

/// Handles every message on `factory.intake` past the daemon's cursor. With `once`, stops there.
/// Otherwise it also runs agents: it starts runs, types their prompts and takes their signals.
pub fn run(factory: &Factory, once: bool) -> Result<(), String> {
    serve(factory, once).map_err(|error| error.to_string())
}

fn serve(factory: &Factory, once: bool) -> Result<(), Error> {
    let mut db = state::open()?;
    let bus = Bus::open_default()?;
    if once {
        drain(&bus, &mut db, factory)?;
        return Ok(());
    }
    let home = runtime::home();
    let listening = runtime::listen(&home)?;
    state::uncertain(&db)?;
    runtime::replay(&home, |line| {
        take(&mut db, line).map_err(|error| error.to_string())
    })?;
    loop {
        drain(&bus, &mut db, factory)?;
        while let Ok(line) = listening.lines.try_recv() {
            apply(&mut db, line)?;
        }
        tend(&mut db, factory, &home)?;
        match listening.lines.recv_timeout(EVERY) {
            Ok(line) => apply(&mut db, line)?,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Err("the signal socket closed".into()),
        }
    }
}

/// Applies a line from the socket, then tells its sender.
fn apply(db: &mut Connection, line: Line) -> Result<(), Error> {
    take(db, &line.text)?;
    let _ = line.applied.send(());
    Ok(())
}

/// Applies one signal line. A line that is not a signal is reported and dropped.
fn take(db: &mut Connection, line: &str) -> Result<(), Error> {
    match serde_json::from_str::<Signal>(line) {
        Ok(signal) => Ok(state::signal(db, &signal)?),
        Err(error) => {
            eprintln!("priory daemon: dropped signal {line:?}: {error}");
            Ok(())
        }
    }
}

/// Ends runs whose tmux session is gone, starts a run if one is admissible, and types prompts.
fn tend(db: &mut Connection, factory: &Factory, home: &Path) -> Result<(), Error> {
    for run in state::live(db)? {
        if runtime::pane(&run.tmux, &run.session).is_none() {
            let reason = format!("run {} exited: tmux session {} is gone", run.id, run.tmux);
            state::block(db, run.task, "exited", &reason)?;
        }
    }
    if let Some(admission) = state::admissible(db, &factory.workflows)?
        && let Err(reason) = start(db, factory, home, &admission)?
    {
        state::block(db, admission.task, "failed", &reason)?;
    }
    while let Some((tmux, session, text)) = state::attempt(db)? {
        let typed = runtime::pane(&tmux, &session)
            .ok_or_else(|| "its pane is gone".to_owned())
            .and_then(|pane| runtime::type_text(&pane, &text));
        if let Err(error) = typed {
            eprintln!("priory daemon: typing into {tmux}: {error}");
        }
    }
    Ok(())
}

/// Starts a run for `admission`. The outer error stops the daemon; the inner one blocks the task.
fn start(
    db: &mut Connection,
    factory: &Factory,
    home: &Path,
    admission: &Admission,
) -> Result<Result<(), String>, Error> {
    let step = factory.workflows[&admission.workflow]
        .value
        .steps
        .iter()
        .find(|step| step.id == admission.step)
        .expect("admissible checked the step");
    let Some(agent) = step
        .agent
        .as_ref()
        .and_then(|name| factory.agents.get(name))
    else {
        return Ok(Err(format!("step {} names no known agent", step.id)));
    };
    let Some(repo) = factory
        .triggers
        .get(&admission.trigger)
        .and_then(|trigger| trigger.value.repo.as_deref())
    else {
        return Ok(Err(format!("trigger {} names no repo", admission.trigger)));
    };
    let (worktree, branch) = match runtime::worktree(home, repo, admission.task) {
        Ok(found) => found,
        Err(reason) => return Ok(Err(reason)),
    };
    let session = uuid::Uuid::new_v4().to_string();
    let tmux = format!("priory-{}", admission.task);
    state::start(
        db,
        &NewRun {
            task: admission.task,
            step: &step.id,
            tmux: &tmux,
            session: &session,
            branch: &branch,
            worktree: &worktree.to_string_lossy(),
            prompt: step.prompt.as_deref().unwrap_or_default(),
        },
    )?;
    let agent = &agent.value;
    Ok(runtime::launch(&Launch {
        tmux: &tmux,
        dir: &worktree,
        home,
        session: &session,
        model: agent.model.as_deref(),
        effort: agent.effort.as_deref(),
        body: &agent.body,
    }))
}

fn drain(bus: &Bus, db: &mut Connection, factory: &Factory) -> Result<(), Error> {
    match bus.connect(CONSUMER, OUTPUT, Start::Beginning) {
        // The intake has not declared its output yet: nothing is pending.
        Err(herdr_bus::Error::UnknownOutput(_)) => return Ok(()),
        connected => connected?,
    };
    loop {
        let deliveries = bus.pull(CONSUMER, 100)?;
        if deliveries.is_empty() {
            return Ok(());
        }
        for delivery in deliveries {
            state::handle(db, &delivery.message, &factory.workflows)?;
            bus.ack(delivery.connection, delivery.seq)?;
        }
    }
}
