{% set metrics = ['event_count'] %}
select
    event_date,
    count(*) as event_count
from {{ ref('fct_events_graph') }}
{% if true %}
group by 1
{% endif %}
