{{ config(materialized='incremental') }}

select distinct
  *,
  row_number() over () as global_row_number
from {{ source('raw', 'events') }} events
cross join {{ ref('stg_events') }} staged

{% if is_incremental() %}
where events.id not in (select id from {{ this }})
{% endif %}

order by events.id
