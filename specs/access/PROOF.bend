import Base
import ./Model.bend as M
import ../Common.bend as C
import ./LAWS.bend as Laws

def Laws.revoked_denied(d, now): {==}

def Laws.deadline_step(g, e):
  match g:
    case M.Grant{d, c, r}:
      match e:
        case M.DelayedReply{}: {==}
        case M.Retry{}: {==}
        case M.TokenRefresh{}: {==}
        case M.RedisHint{}: {==}
        case M.DirectorySuccess{}: {==}
        case M.Revoke{}: {==}
        case M.ClockFailure{}: {==}

def Laws.deadline_trace(es, g):
  match es:
    case Nil{}: {==}
    case Con{+e, tail}:
      Equal.trans(Nat, M.deadline(M.replay(tail, M.step(g, e))), M.deadline(M.step(g, e)), M.deadline(g), Laws.deadline_trace(tail, M.step(g, e)), Laws.deadline_step(g, e))

def Laws.directory_cannot_unrevoke(d): {==}

def Laws.clock_failure_denies(d, r, now): {==}

def Laws.deadline_exclusive(d): C.lt_self(d)

def Laws.before_deadline(): {==}
