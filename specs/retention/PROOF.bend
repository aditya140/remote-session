import Base
import ./Model.bend as M
import ../Common.bend as C
import ./LAWS.bend as Laws

def Laws.hold_a_prevents_purge(b, age): {==}

def Laws.remaining_hold_survives_release(age): {==}

def Laws.eligible_purge(): {==}

def Laws.hold_cannot_restore_purged_content(): {==}

def Laws.upload_does_not_reset_age(s):
  match s:
    case M.Item{x, a, b, age}: {==}
