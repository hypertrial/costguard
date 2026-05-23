{{ config(materialized='incremental', unique_key='tx_hash') }}
select s.tx_hash, s.block_time
from {{ source('dex', 'trades') }} s
{% if is_incremental() %}
join {{ this }} t on s.tx_hash = t.tx_hash and s.block_time >= date_sub(current_date(), interval 3 day)
{% endif %}
