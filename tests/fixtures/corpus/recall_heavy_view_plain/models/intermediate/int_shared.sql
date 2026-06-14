{{ config(materialized='view') }} select id, event_date from base
