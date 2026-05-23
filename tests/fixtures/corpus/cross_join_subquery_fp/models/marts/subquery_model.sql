select *
from (select a, b from {{ ref('stg_base') }}), {{ ref('stg_side') }} y
