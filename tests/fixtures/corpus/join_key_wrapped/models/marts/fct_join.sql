select a.id, b.id
from {{ ref('stg_a') }} a
join {{ ref('stg_b') }} b on lower(a.email) = cast(b.id as varchar)
