with expensive as (
  select
    id,
    lower(trim(email)) as normalized_email
  from {{ ref('stg_events') }}
)

select
  a.id,
  lower(trim(a.normalized_email)) as first_email,
  lower(trim(a.normalized_email)) as second_email
from expensive a
join expensive b on a.id = b.id
