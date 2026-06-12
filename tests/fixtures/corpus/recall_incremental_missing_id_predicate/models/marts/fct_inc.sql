{{ config(materialized='incremental', unique_key='id') }}
select id from {{ source('raw', 'events') }}
{% if is_incremental() %}
where id > 1
{% endif %}
