select tx_hash
from {{ ref('stg_events') }}
where date_trunc('day', block_time) >= current_date - interval '7' day
