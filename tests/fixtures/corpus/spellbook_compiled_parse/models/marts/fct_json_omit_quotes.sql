{{ config(materialized='table') }}

select try_cast(json_query(payload, 'lax $.revocable' omit quotes) as boolean) as is_revocable
from {{ source('dex', 'trades') }}
