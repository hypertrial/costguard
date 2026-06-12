{{ config(materialized='incremental', unique_key='id') }}
select id from {{ ref('stg_events') }}
