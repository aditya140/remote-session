import Base
import ./Model.bend as M
import ../Common.bend as C
import ./LAWS.bend as Laws

def Laws.network_no_kill(e):
  match e:
    case M.NetDetach{}: {==}
    case M.NetExit{}: {==}
    case M.NetPause{}: {==}
    case M.NetRevoke{}: {==}

def Laws.network_trace_no_kill(es):
  match es:
    case Nil{}: {==}
    case Con{+e, tail}:
      match e:
        case M.NetDetach{}: Laws.network_trace_no_kill(tail)
        case M.NetExit{}: Laws.network_trace_no_kill(tail)
        case M.NetPause{}: Laws.network_trace_no_kill(tail)
        case M.NetRevoke{}: Laws.network_trace_no_kill(tail)

def Laws.explicit_termination(): {==}

def Laws.no_confirmation_no_kill(): {==}

def Laws.owner_loss_unknown(): {==}
