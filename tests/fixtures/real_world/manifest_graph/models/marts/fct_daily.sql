select
    event_date,
    count(*) as event_count
from {{ ref('fct_events') }}
group by 1
