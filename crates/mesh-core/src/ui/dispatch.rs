//! UI command dispatch (DESIGN §3.2, §3.7): base commands and Lab commands are handled
//! here and in `lab/`; everything else goes to the registered modules in order. A
//! command no module answers is refused with the error that explains why (no session,
//! wrong mode, or not supported by this build).

use mesh_api::{AudioCheckOp, CommandResult, CommandType, ErrorCode, UiCommand};

use crate::effects::AudioCommand;
use crate::engine::{Engine, EngineMode};

/// Commands that work without a session (group management, stored data).
fn sessionless(t: CommandType) -> bool {
    matches!(
        t,
        CommandType::CreateGroup
            | CommandType::ParseInvite
            | CommandType::JoinGroup
            | CommandType::ListGroups
            | CommandType::ForgetGroup
            | CommandType::GetInvite
            | CommandType::SetPrompts
            | CommandType::SetDisplayName
            | CommandType::SetVehicle
            | CommandType::SetBuiltinMicTx
            | CommandType::SetVoxSensitivity
            | CommandType::SetPttStyle
            | CommandType::AudioCheck
            | CommandType::SetRoutePreference
    )
}

/// Handles one command.
pub(crate) fn dispatch(e: &mut Engine, cmd: &UiCommand) -> CommandResult {
    let t = cmd.command_type();
    match cmd {
        UiCommand::RequestSnapshot {} => {
            e.ctx.ui.request();
            return CommandResult::done();
        }
        UiCommand::LabStart { .. }
        | UiCommand::LabStop {}
        | UiCommand::LabMark { .. }
        | UiCommand::LabListRuns {}
        | UiCommand::LabDeleteRun { .. } => {
            return e.lab.on_command(&mut e.ctx, cmd);
        }
        UiCommand::AudioCheck(AudioCheckOp::Start { .. }) if e.lab.run_active() => {
            // An audio check is refused during a Lab run (§3.2).
            return Engine::refuse(ErrorCode::LabRunActive);
        }
        _ => {}
    }
    if e.ctx.is_lab() && !sessionless(t) {
        return Engine::refuse(ErrorCode::WrongMode);
    }
    let ctx = &mut e.ctx;
    if let Some(r) = e.modules.first(|m| m.on_command(ctx, cmd)) {
        return r;
    }
    // Base fallbacks when no module took the command.
    match cmd {
        UiCommand::SetRoutePreference { pref } => {
            e.ctx.fx.audio(AudioCommand::SetRoutePreference(*pref));
            CommandResult::done()
        }
        _ if sessionless(t) => Engine::refuse(ErrorCode::NotSupported),
        _ => match e.ctx.mode {
            EngineMode::Session { .. } => Engine::refuse(ErrorCode::NotSupported),
            EngineMode::Lab { .. } => Engine::refuse(ErrorCode::WrongMode),
            EngineMode::Idle => Engine::refuse(ErrorCode::NoSession),
        },
    }
}
