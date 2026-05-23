select t.hash
from {{ ref('stg_events') }} t
cross join date_ranges dr
where t.block_time between dr.start_time and dr.end_time
