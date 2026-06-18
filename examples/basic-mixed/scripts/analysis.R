library(tidyverse)
library(reticulate)

py_config()

result <- tibble(
  language = c("R", "Python"),
  ready = c(TRUE, py_available(initialize = TRUE))
)

dir.create("results", showWarnings = FALSE)
write_csv(result, "results/languages.csv")
print(result)
