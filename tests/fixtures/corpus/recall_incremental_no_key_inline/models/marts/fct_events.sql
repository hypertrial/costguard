{{ config(materialized='incremental') }}
select id from {{ ref('stg_events') }}
