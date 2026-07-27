{{ config(materialized='incremental', unique_key='id') }}

select id, updated_at
from {{ source('raw', 'events') }}
{% if is_incremental() %}
where id > 1
{% endif %}
