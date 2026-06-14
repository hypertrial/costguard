{{ config(materialized='incremental', incremental_strategy='merge', unique_key='id', incremental_predicates=["DBT_INTERNAL_DEST.updated_at >= current_date - 3"]) }}
select id, updated_at from events
