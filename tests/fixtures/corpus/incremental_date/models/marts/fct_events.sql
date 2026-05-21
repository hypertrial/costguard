{{ config(materialized='incremental', unique_key='id') }}

select id, updated_at
from {{ source('raw', 'events') }}
{% if is_incremental() %}
where updated_at > (select max(updated_at) from {{ this }})
{% endif %}
