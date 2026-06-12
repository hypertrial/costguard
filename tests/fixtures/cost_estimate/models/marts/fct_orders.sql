{{
    config(
        materialized='table'
    )
}}

with base as (
    select id, event_type from {{ source('raw', 'events') }}
),
dedup as (
    select * from base
),
again as (
    select * from dedup
),
final as (
    select * from again
)
select * from final
