import Base
import ./Model.bend as M
import ../Common.bend as C
import ./LAWS.bend as Laws

def Laws.non_durable_receipt_retained(): {==}

def Laws.retry_immutable(c): {==}

def Laws.wrong_epoch_retained(i, d):
  match i:
    case False{}: {==}
    case True{}: {==}

def Laws.wrong_identity_retained(e, d): {==}

def Laws.valid_receipt_reclaims(): {==}

def Laws.capacity_records_gap(): {==}

def Laws.failed_metadata_is_not_durable_gap(): {==}

def Laws.storage_failure_keeps_execution(e): {==}
