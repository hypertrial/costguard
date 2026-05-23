{{
    config(
        materialized='incremental',
        unique_key=['transfer_id'],
        incremental_predicates=[incremental_predicate('DBT_INTERNAL_DEST.evt_block_time')]
    )
}}

with source_data as (
    select evt_block_time from {{ source('dex', 'trades') }}
)

select * from source_data
{% if is_incremental() %}
where {{ incremental_predicate('p.minute') }}
{% endif %}
