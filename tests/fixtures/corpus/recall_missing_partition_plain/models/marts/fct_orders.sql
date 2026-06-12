{{ config(materialized='incremental', unique_key='order_id') }}
select order_id from {{ ref('stg_orders') }}
