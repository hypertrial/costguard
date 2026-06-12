{{ config(materialized='incremental', unique_key='id', on_schema_change='sync_all_columns') }}
select id from {{ ref('stg_events') }}
