-- owner: {{ var('owner') }}
{{ config(materialized='incremental', unique_key='id') }}
with raw as (select * from {{ source('events', 'raw_events') }}),
filtered as (select * from raw {% if is_incremental() %} where event_date >= current_date - 3 {% endif %})
select * from filtered
