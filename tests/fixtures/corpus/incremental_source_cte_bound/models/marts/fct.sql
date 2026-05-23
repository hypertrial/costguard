{{ config(materialized='incremental', unique_key='tx_hash') }}
with filtered as (
  select tx_hash, block_time
  from {{ source('dex', 'trades') }}
  where block_time >= date_sub(current_date(), interval 3 day)
)
select * from filtered
{% if is_incremental() %}
where block_time >= (select max(block_time) from {{ this }})
{% endif %}
