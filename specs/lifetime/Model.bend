import Base

type StepEvent is Data:
  Detach{}
  ClientExit{}
  ConnectorLoss{}
  Revoke{}
  Interrupt{}
  Terminate{confirmed: Bool}
  NativeExit{}
  OwnerCrash{}

type Effect is Data:
  Keep{}
  InterruptTurn{}
  Kill{}
  ObserveExit{}
  ReconcileUnknown{}

def effect(e: StepEvent) -> Effect:
  match e:
    case Interrupt{}: InterruptTurn{}
    case Terminate{True{}}: Kill{}
    case NativeExit{}: ObserveExit{}
    case OwnerCrash{}: ReconcileUnknown{}
    case _: Keep{}

def is_kill(e: Effect) -> Bool:
  match e:
    case Kill{}: True{}
    case _: False{}

type NetworkEvent is Data:
  NetDetach{}
  NetExit{}
  NetPause{}
  NetRevoke{}

def network(e: NetworkEvent) -> StepEvent:
  match e:
    case NetDetach{}: Detach{}
    case NetExit{}: ClientExit{}
    case NetPause{}: ConnectorLoss{}
    case NetRevoke{}: Revoke{}

def kills(es: List<&2, NetworkEvent>) -> Bool:
  match es:
    case Nil{}: False{}
    case Con{+e, tail}: Bool.or(is_kill(effect(network(e))), kills(tail))
