select event_date, count(*) as event_count
from {{ ref('fct_events_graph') }}
group by 1
