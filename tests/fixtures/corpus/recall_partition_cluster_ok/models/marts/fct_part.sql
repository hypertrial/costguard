{{ config(materialized='incremental', unique_key='id', partition_by={'field': 'event_date'}) }}
select id, event_date from {{ ref('stg_events') }}
