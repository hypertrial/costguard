{{ config(materialized='incremental', unique_key='id') }}
select id from {{ source('raw', 'events') }}
{% if is_incremental() %}
where event_date >= current_date - 3
{% endif %}
