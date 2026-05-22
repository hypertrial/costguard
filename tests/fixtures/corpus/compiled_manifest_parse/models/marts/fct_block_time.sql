{{ config(materialized='incremental', unique_key='tx_hash') }}

{% set lookback_days = 3 %}

select
    tx_hash,
    block_time
from {{ source('dex', 'trades') }}
where block_time >= date_sub(current_date, interval '{{ lookback_days }}' day)
{% if is_incremental() %}
  and block_time > (select max(block_time) from {{ this }})
{% endif %}
