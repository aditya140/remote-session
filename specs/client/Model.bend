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
