import Base
import ./Model.bend as M
import ../Common.bend as C
import ./LAWS.bend as Laws

def Laws.snapshot_boundary(s): {==}

def Laws.next_event_applies(s):
  match s:
    case 0n: {==}
    case 1n+p: Laws.next_event_applies(p)

def Laws.snapshot_not_reapplied(s):
  match s:
    case 0n: {==}
    case 1n+p: Laws.snapshot_not_reapplied(p)

def Laws.gap_resync(s):
  match s:
    case 0n: {==}
    case 1n+p: Laws.gap_resync(p)

def Laws.overflow_resync(): {==}

def Laws.lost_final_event_detected(s): C.le_self(1n+s)
