{{ config(materialized='table') }} select id, event_date from {{ ref('stg_events') }}
