from pathlib import Path

import pandas as pd
from sklearn.linear_model import LinearRegression

data = pd.DataFrame({"x": [1, 2, 3, 4], "y": [2, 4, 6, 8]})
model = LinearRegression().fit(data[["x"]], data["y"])

Path("results").mkdir(exist_ok=True)
Path("results/model.txt").write_text(
    f"coefficient={model.coef_[0]:.1f}\n", encoding="utf-8"
)
print(f"coefficient: {model.coef_[0]:.1f}")
