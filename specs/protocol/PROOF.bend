import Base
import ./Model.bend as M
import ../Common.bend as C
import ./LAWS.bend as Laws

def Laws.round_trip(m):
  match m:
    case M.Output{s, n}: {==}
    case M.Input{s, n, h, c}: {==}

def Laws.invalid_frame_no_dispatch(ts): {==}

def Laws.truncated_input_denied(s, n, h): {==}

def Laws.no_tag_no_dispatch(n): {==}
