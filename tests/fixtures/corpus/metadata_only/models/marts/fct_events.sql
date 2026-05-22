{{ config(materialized='incremental', unique_key='id') }}

select 1 as id
from {{ ref('stg_events') }}
{% if is_incremental() %}
where updated_at > current_date
{% endif %}
