{{ config(materialized='table') }}

select
    tx_hash,
    TRY_CAST(block_time as timestamp) as block_time
from {{ source('dex', 'trades') }}
