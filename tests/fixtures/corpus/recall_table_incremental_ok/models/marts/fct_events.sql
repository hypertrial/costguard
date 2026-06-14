{{ config(materialized='incremental', unique_key='id') }} select id, event_date from events
