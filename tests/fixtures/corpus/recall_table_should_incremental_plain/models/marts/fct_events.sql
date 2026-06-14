{{ config(materialized='table') }} select id, event_date from events
