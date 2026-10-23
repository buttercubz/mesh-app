// Runtime validators for `CommandResult` (crates/mesh-api/src/result.rs): an untagged
// union of exact payload objects, tried in the Rust declaration order.
import type {
  CommandError,
  CommandResult,
  GroupSummary,
  NodeRef,
  OkAccessLoss,
  OkDone,
  OkGroupId,
  OkGroups,
  OkInvite,
  OkInviteInfo,
  OkLabRun,
  OkLabRuns,
  ResultKind,
} from "../generated/index.js";
import { ERROR_CODES, MEMBER_ROLES, VEHICLES } from "../generated/index.js";
import type { Validator } from "../validate.js";
import { anyOf, arr, bool, lit, nullable, obj, oneOf, str, u16, u32, u8 } from "../validate.js";
import { labRunInfo } from "./lab.js";

export const commandError = obj<CommandError>({ ok: lit(false), code: oneOf(ERROR_CODES) });

export const nodeRef = obj<NodeRef>({ nodeId: u16(), name: str() });

export const groupSummary = obj<GroupSummary>({
  groupId: str(),
  name: str(),
  role: oneOf(MEMBER_ROLES),
  vehicle: oneOf(VEHICLES),
  members: u8(),
  epoch: u32(),
  closed: bool(),
});

/** Successful payload per result kind (`COMMAND_RESULT_KINDS` maps commands to kinds). */
export interface ResultPayloads {
  done: OkDone;
  groupId: OkGroupId;
  inviteInfo: OkInviteInfo;
  groups: OkGroups;
  invite: OkInvite;
  accessLoss: OkAccessLoss;
  labRun: OkLabRun;
  labRuns: OkLabRuns;
}

const ok = lit(true);

export const resultPayloads: { readonly [K in ResultKind]: Validator<ResultPayloads[K]> } = {
  done: obj<OkDone>({ ok }),
  groupId: obj<OkGroupId>({ ok, groupId: str() }),
  inviteInfo: obj<OkInviteInfo>({
    ok,
    valid: bool(),
    groupName: nullable(str()),
    groupId: nullable(str()),
    epoch: nullable(u32()),
    expired: bool(),
    alreadyMember: bool(),
    leaderSigOk: bool(),
  }),
  groups: obj<OkGroups>({ ok, groups: arr(groupSummary) }),
  invite: obj<OkInvite>({ ok, qrPng: str(), hideAfterMs: u32() }),
  accessLoss: obj<OkAccessLoss>({ ok, unconfirmed: arr(nodeRef) }),
  labRun: obj<OkLabRun>({ ok, runId: str() }),
  labRuns: obj<OkLabRuns>({ ok, runs: arr(labRunInfo) }),
};

export const commandResult: Validator<CommandResult> = anyOf<CommandResult>("CommandResult", [
  commandError,
  resultPayloads.groupId,
  resultPayloads.inviteInfo,
  resultPayloads.groups,
  resultPayloads.invite,
  resultPayloads.accessLoss,
  resultPayloads.labRun,
  resultPayloads.labRuns,
  resultPayloads.done,
]);
