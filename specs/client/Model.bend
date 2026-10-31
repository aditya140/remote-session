import Base
import ../Common.bend as C

type Action is Data:
  KillSession{}
  TakeControl{}

def same_action(a: Action, b: Action) -> Bool:
  match a b:
    case KillSession{} KillSession{}: True{}
    case TakeControl{} TakeControl{}: True{}
    case _ _: False{}

type Confirmation is Data:
  Confirmation{target: Nat, action: Action}

def may_execute(visible: Bool, current_target: Nat, requested: Action, confirmation: Confirmation) -> Bool:
  match confirmation:
    case Confirmation{target, action}:
      Bool.and(visible, Bool.and(same_action(requested, action), C.eq(current_target, target)))

type Outcome is Data:
  NotSent{}
  Rejected{}
  OutcomeUnknown{}
  Applied{}

def success(o: Outcome) -> Bool:
  match o:
    case Applied{}: True{}
    case _: False{}

def reconnect_resends(o: Outcome) -> Bool:
  False{}

# This classifies connection recovery, never application-operation retries.
type ConnectionFailure is Data:
  AccessStale{}
  Overloaded{}
  Fatal{}

def can_reconnect(failure: ConnectionFailure) -> Bool:
  match failure:
    case AccessStale{}: True{}
    case Overloaded{}: True{}
    case Fatal{}: False{}

# Caller-owned credential channels. Fixed injection retains its own publisher.
def credential_valid(publisher_live: Bool, sign_in_required: Bool, expired: Bool) -> Bool:
  Bool.and(publisher_live, Bool.and(Bool.not(sign_in_required), Bool.not(expired)))

type TurnStatus is Data:
  NativeCompleted{}
  NativeFailed{}
  NativeInterrupted{}
  LegacyUnknown{}

type TurnResult is Data:
  Completed{}
  Failed{}
  Interrupted{}
  HistoryUnavailable{}

type Completion is Data:
  Completion{turn: Nat, status: TurnStatus}

def terminal_result(status: TurnStatus) -> TurnResult:
  match status:
    case NativeCompleted{}: Completed{}
    case NativeFailed{}: Failed{}
    case NativeInterrupted{}: Interrupted{}
    case LegacyUnknown{}: HistoryUnavailable{}

def choose_completion(matches: Bool, status: TurnStatus, older: Maybe<TurnResult>) -> Maybe<TurnResult>:
  match matches:
    case True{}: Some{terminal_result(status)}
    case False{}: older

# The list contains terminal evidence, newest first, within one verified session.
def latest_completion(+target: Nat, events: List<&2, Completion>) -> Maybe<TurnResult>:
  match events:
    case Nil{}: None{}
    case Con{Completion{turn, status}, tail}:
      choose_completion(C.eq(target, turn), status, latest_completion(target, tail))
