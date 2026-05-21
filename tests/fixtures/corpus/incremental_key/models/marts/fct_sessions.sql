{{ config(materialized='incremental', unique_key='id') }}

select id
from {{ source('raw', 'events') }} events
{% if is_incremental() %}
where events.id not in (select id from {{ this }})
{% endif %}
