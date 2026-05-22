{{ config(materialized='table') }}

select current_timestamp as _updated_at from {{ source('dex', 'trades') }}
