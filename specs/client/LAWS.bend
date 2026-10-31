import Base
import ./Model.bend as M
import ../Common.bend as C

# CLIENT-001
law hidden_selection_cannot_act:
  for current: Nat
  for action: M.Action
  for confirmation: M.Confirmation
  {M.may_execute(False{}, current, action, confirmation) == False{} : Bool}

# CLIENT-002
law confirmation_cannot_change_action:
  for current: Nat
  for target: Nat
  {M.may_execute(True{}, current, M.KillSession{}, M.Confirmation{target, M.TakeControl{}}) == False{} : Bool}

# CLIENT-003
law matching_confirmation_allows:
  for +target: Nat
  {M.may_execute(True{}, target, M.KillSession{}, M.Confirmation{target, M.KillSession{}}) == True{} : Bool}

# CLIENT-004
law unknown_not_success:
  {M.success(M.OutcomeUnknown{}) == False{} : Bool}

# CLIENT-005
law reconnect_never_resends_input:
  for o: M.Outcome
  {M.reconnect_resends(o) == False{} : Bool}

# CLIENT-002: A confirmation for a different target never authorizes the action.
law changed_target_denied:
  for +current: Nat
  for +target: Nat
  for different: {C.eq(current, target) == False{} : Bool}
  {M.may_execute(True{}, current, M.KillSession{}, M.Confirmation{target, M.KillSession{}}) == False{} : Bool}

# CLIENT-006: Only temporary connection failures are eligible for recovery.
law stale_connection_can_reconnect:
  {M.can_reconnect(M.AccessStale{}) == True{} : Bool}

law overloaded_connection_can_reconnect:
  {M.can_reconnect(M.Overloaded{}) == True{} : Bool}

law fatal_connection_stops:
  {M.can_reconnect(M.Fatal{}) == False{} : Bool}

# CLIENT-007: Credential ownership and validity are independent requirements.
law credential_owner_loss_denies:
  for sign_in: Bool
  for expired: Bool
  {M.credential_valid(False{}, sign_in, expired) == False{} : Bool}

law sign_in_required_denies:
  for expired: Bool
  {M.credential_valid(True{}, True{}, expired) == False{} : Bool}

law expired_credential_denies:
  {M.credential_valid(True{}, False{}, True{}) == False{} : Bool}

law current_credential_allows:
  {M.credential_valid(True{}, False{}, False{}) == True{} : Bool}

# CLIENT-008: A terminal event is not necessarily a successful turn.
law completed_turn_succeeds:
  {M.terminal_result(M.NativeCompleted{}) == M.Completed{} : M.TurnResult}

law failed_turn_is_not_success:
  {M.terminal_result(M.NativeFailed{}) == M.Failed{} : M.TurnResult}

law interrupted_turn_is_not_success:
  {M.terminal_result(M.NativeInterrupted{}) == M.Interrupted{} : M.TurnResult}

law legacy_turn_is_not_success:
  {M.terminal_result(M.LegacyUnknown{}) == M.HistoryUnavailable{} : M.TurnResult}

# CLIENT-009: Only the latest matching evidence establishes the turn result.
law absent_completion_is_not_evidence:
  for target: Nat
  {M.latest_completion(target, Nil{}) == None{} : Maybe<M.TurnResult>}

law newest_matching_completion_wins:
  for +target: Nat
  for +status: M.TurnStatus
  for older: List<&2, M.Completion>
  {M.latest_completion(target, Con{M.Completion{target, status}, older}) == Some{M.terminal_result(status)} : Maybe<M.TurnResult>}

law different_turn_is_skipped:
  for +target: Nat
  for +other: Nat
  for status: M.TurnStatus
  for +older: List<&2, M.Completion>
  for different: {C.eq(target, other) == False{} : Bool}
  {M.latest_completion(target, Con{M.Completion{other, status}, older}) == M.latest_completion(target, older) : Maybe<M.TurnResult>}
