select
    id,
    event_date,
    payload
from {{ source('raw', 'events') }}
