def model(dbt, session):
    df = dbt.ref("stg_events")
    df["label"] = df.apply(lambda row: str(row["id"]), axis=1)
    return df
