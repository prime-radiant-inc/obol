package main

import (
	"fmt"
	"os"
	"strconv"

	"github.com/prime-radiant-inc/obol/bindings/go/obol"
)

func main() {
	if len(os.Args) < 2 {
		fmt.Fprintln(os.Stderr, "usage: total <transcript> [dialect]")
		os.Exit(2)
	}
	dialect := ""
	if len(os.Args) >= 3 {
		dialect = os.Args[2]
	}
	est, err := obol.EstimatePath(os.Args[1], dialect)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	fmt.Println(strconv.FormatFloat(est.TotalUSD, 'g', -1, 64))
}
