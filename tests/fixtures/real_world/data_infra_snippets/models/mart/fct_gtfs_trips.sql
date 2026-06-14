{{ config(
    materialized='table',
    cluster_by='service_date',
    tags=['gtfs', 'data_infra_snippet']
) }}

select
    trip_id,
    route_id,
    service_date,
    trip_id as trip_id_dup,
    start_time,
    end_time
from {{ source('gtfs', 'trips') }}
