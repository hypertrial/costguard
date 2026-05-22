{{ config(materialized='table') }}

select b.epoch
from {{ ref('fct_underscore_alias') }} b
left join {{ ref('fct_underscore_alias') }} start on start._updated_at = b._updated_at
