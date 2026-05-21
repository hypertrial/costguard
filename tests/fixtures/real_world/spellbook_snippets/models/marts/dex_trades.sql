{{ config(
    materialized='incremental',
    unique_key='tx_hash',
    tags=['dex', 'spellbook_snippet']
) }}

{% set blockchain = var('blockchain', 'ethereum') %}

select
    '{{ blockchain }}' as blockchain,
    tx_hash,
    json_extract_scalar(raw_data, '$.token_in') as token_in,
    json_extract_scalar(raw_data, '$.token_in') as token_in_dup,
    block_time
from {{ source('dex', 'trades') }}
{% if is_incremental() %}
where block_time >= date_sub(current_date(), interval 3 day)
{% endif %}
