select 'cross join in string' as note, 'a cross join b' as example
from {{ ref('stg_orders') }}
