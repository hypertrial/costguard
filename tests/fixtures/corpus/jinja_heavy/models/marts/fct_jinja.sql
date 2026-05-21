{% set columns = ['id', 'payload'] %}
{{ config(materialized='table') }}

select
  {% for column in columns %}
  {{ column }}{% if not loop.last %},{% endif %}
  {% endfor %}
from {{ source('raw', 'events') }}
