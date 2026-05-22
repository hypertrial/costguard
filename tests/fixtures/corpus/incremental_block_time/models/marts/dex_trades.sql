{{ config(
    materialized='incremental',
    unique_key='tx_hash'
) }}

select
    tx_hash,
    block_time
from {{ source('dex', 'trades') }}
{% if is_incremental() %}
where block_time >= date_sub(current_date(), interval 3 day)
{% endif %}
