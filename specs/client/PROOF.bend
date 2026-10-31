import Base
import ./Model.bend as M
import ../Common.bend as C
import ./LAWS.bend as Laws

def Laws.hidden_selection_cannot_act(current, action, confirmation):
  match confirmation:
    case M.Confirmation{t, a}: {==}

def Laws.confirmation_cannot_change_action(current, target): {==}

def Laws.matching_confirmation_allows(target): C.eq_self(target)

def Laws.unknown_not_success(): {==}

def Laws.reconnect_never_resends_input(o): {==}

def Laws.changed_target_denied(current, target, different): different

def Laws.stale_connection_can_reconnect(): {==}
def Laws.overloaded_connection_can_reconnect(): {==}
def Laws.fatal_connection_stops(): {==}
def Laws.credential_owner_loss_denies(sign_in, expired): {==}
def Laws.sign_in_required_denies(expired): {==}
def Laws.expired_credential_denies(): {==}
def Laws.current_credential_allows(): {==}
def Laws.completed_turn_succeeds(): {==}
def Laws.failed_turn_is_not_success(): {==}
def Laws.interrupted_turn_is_not_success(): {==}
def Laws.legacy_turn_is_not_success(): {==}
def Laws.absent_completion_is_not_evidence(target): {==}

def Laws.newest_matching_completion_wins(target, status, older):
  %Equal.sym(Bool, C.eq(target, target), True{}, C.eq_self(target)) : {M.choose_completion(_, status, M.latest_completion(target, older)) == Some{M.terminal_result(status)} : Maybe<M.TurnResult>}
  {==}

def Laws.different_turn_is_skipped(target, other, status, older, different):
  %Equal.sym(Bool, C.eq(target, other), False{}, different) : {M.choose_completion(_, status, M.latest_completion(target, older)) == M.latest_completion(target, older) : Maybe<M.TurnResult>}
  {==}
