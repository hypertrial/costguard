{{ config(materialized='incremental', incremental_strategy='merge', unique_key='id') }}
select id, updated_at from {{ ref('stg_events') }}
{% if is_incremental() %}
where updated_at >= current_date - 3
{% endif %}
