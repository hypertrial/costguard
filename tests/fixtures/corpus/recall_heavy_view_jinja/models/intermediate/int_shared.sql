{{ config(materialized='view') }} select id, event_date from {{ ref('stg_base') }}
