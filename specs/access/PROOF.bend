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

def Laws.revocation_step(d, c, e):
  match e:
    case M.DelayedReply{}: {==}
    case M.Retry{}: {==}
    case M.TokenRefresh{}: {==}
    case M.RedisHint{}: {==}
    case M.DirectorySuccess{}: {==}
    case M.Revoke{}: {==}
    case M.ClockFailure{}: {==}

def Laws.revocation_trace(es, d, c):
  match es:
    case Nil{}: {==}
    case Con{e, tail}:
      match e:
        case M.DelayedReply{}: Laws.revocation_trace(tail, d, c)
        case M.Retry{}: Laws.revocation_trace(tail, d, c)
        case M.TokenRefresh{}: Laws.revocation_trace(tail, d, c)
        case M.RedisHint{}: Laws.revocation_trace(tail, d, c)
        case M.DirectorySuccess{}: Laws.revocation_trace(tail, d, c)
        case M.Revoke{}: Laws.revocation_trace(tail, d, c)
        case M.ClockFailure{}: Laws.revocation_trace(tail, d, False{})

def Laws.failed_clock_step(d, r, e):
  match e:
    case M.DelayedReply{}: {==}
    case M.Retry{}: {==}
    case M.TokenRefresh{}: {==}
    case M.RedisHint{}: {==}
    case M.DirectorySuccess{}: {==}
    case M.Revoke{}: {==}
    case M.ClockFailure{}: {==}

def Laws.failed_clock_trace(es, d, r):
  match es:
    case Nil{}: {==}
    case Con{e, tail}:
      match e:
        case M.DelayedReply{}: Laws.failed_clock_trace(tail, d, r)
        case M.Retry{}: Laws.failed_clock_trace(tail, d, r)
        case M.TokenRefresh{}: Laws.failed_clock_trace(tail, d, r)
        case M.RedisHint{}: Laws.failed_clock_trace(tail, d, r)
        case M.DirectorySuccess{}: Laws.failed_clock_trace(tail, d, r)
        case M.Revoke{}: Laws.failed_clock_trace(tail, d, True{})
        case M.ClockFailure{}: Laws.failed_clock_trace(tail, d, r)
