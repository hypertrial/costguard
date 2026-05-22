{{ config(
    materialized='incremental',
    incremental_strategy='append'
) }}

select id, block_time
from {{ source('raw', 'events') }}
{% if is_incremental() %}
where block_time > (select max(block_time) from {{ this }})
{% endif %}
