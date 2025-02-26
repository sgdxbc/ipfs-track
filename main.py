import polars as pl


def main():
    df = pl.read_csv(
        "data/*/*.csv",
        new_columns=["elapsed", "target", "op", "count"],
        has_header=False,
    )
    df_hours = (
        df.group_by(pl.col("op"), (pl.col("elapsed") // 3600).alias("hour"))
        .agg(pl.col("count").mean())
        .sort("hour")
    )
    print(df_hours)
    df_hours.plot.line(x="hour", y="count", color="op").save("count.png")


if __name__ == "__main__":
    main()
