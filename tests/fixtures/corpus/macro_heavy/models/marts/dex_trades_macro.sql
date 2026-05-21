{{ config(
    materialized='incremental',
    unique_key='tx_hash'
) }}

{% set start_blocktime = '2024-01-01' %}
{% set end_blocktime = '2024-01-02' %}

select
    tx_hash,
    block_time,
    {{ dbt_utils.star(from=source('dex', 'trades'), except=['raw_data']) }}
from {{ source('dex', 'trades') }}
where block_time between '{{ start_blocktime }}' and '{{ end_blocktime }}'
{% if is_incremental() %}
  and block_time > (select max(block_time) from {{ this }})
{% endif %}
