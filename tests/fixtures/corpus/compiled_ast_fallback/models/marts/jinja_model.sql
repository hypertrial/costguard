{% set cols = ['id', 'block_time'] %}
select {{ cols | join(', ') }}
from {{ ref('stg_events') }}
where date_trunc('day', block_time) >= current_date - interval '7' day
