{{ config(materialized='incremental', unique_key='id', full_refresh=true) }}
select id from {{ ref('stg_events') }}
