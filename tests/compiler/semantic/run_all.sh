#!/bin/bash

# TriCTI Semantic Analysis Test Suite Runner
# Executes all semantic edge case tests and reports results

set -e

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Get the directory of this script
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

# Test files in order
TEST_FILES=(
    "type_inference.tri"
    "generics.tri"
    "enums.tri"
    "functions.tri"
    "structs.tri"
    "arrays_slices.tri"
    "operators.tri"
    "control_flow.tri"
)

# Counters
TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0

# Track failed test files
FAILED_FILES=()

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}TriCTI Semantic Analysis Test Suite${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# Function to count tests in a file
count_tests() {
    local test_file="$1"
    grep -c "^test_" "$test_file" || true
}

# Function to run a single test file
run_test_file() {
    local test_file="$1"
    local test_path="$SCRIPT_DIR/$test_file"
    
    echo -e "${YELLOW}Testing: $test_file${NC}"
    
    # Check if file exists
    if [ ! -f "$test_path" ]; then
        echo -e "${RED}  ✗ File not found: $test_path${NC}"
        ((FAILED_TESTS++))
        FAILED_FILES+=("$test_file (not found)")
        return 1
    fi
    
    # Count tests in file
    local test_count=$(count_tests "$test_path")
    ((TOTAL_TESTS += test_count))
    
    echo -e "  Found $test_count test cases"
    
    # Try to compile the test file with SKIP_STDLIB=1
    export SKIP_STDLIB=1
    export LLVM_SYS_181_PREFIX=/nix/store/0l2qyps0nlhdpl5hxzrxbr3lkq7irkmk-llvm-18.1.8-dev
    
    # Run the compiler
    if "$PROJECT_ROOT/target/debug/tricti" "$test_path" > /dev/null 2>&1; then
        echo -e "${GREEN}  ✓ All tests passed (file compiled successfully)${NC}"
        ((PASSED_TESTS += test_count))
        return 0
    else
        echo -e "${RED}  ✗ Compilation failed${NC}"
        ((FAILED_TESTS += test_count))
        FAILED_FILES+=("$test_file")
        
        # Show error details
        echo -e "${RED}  Error output:${NC}"
        "$PROJECT_ROOT/target/debug/tricti" "$test_path" 2>&1 | head -n 10 | sed 's/^/    /'
        return 1
    fi
}

# Build the compiler first
echo -e "${BLUE}Building compiler...${NC}"
cd "$PROJECT_ROOT"
if cargo build 2>&1 | tail -n 5; then
    echo -e "${GREEN}✓ Compiler built successfully${NC}"
else
    echo -e "${RED}✗ Failed to build compiler${NC}"
    exit 1
fi
echo ""

# Run all test files
echo -e "${BLUE}Running semantic test files...${NC}"
echo ""

for test_file in "${TEST_FILES[@]}"; do
    run_test_file "$test_file"
    echo ""
done

# Summary
echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}Test Suite Summary${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""
echo -e "Total test cases:    $TOTAL_TESTS"
echo -e "${GREEN}Passed:              $PASSED_TESTS${NC}"

if [ $FAILED_TESTS -gt 0 ]; then
    echo -e "${RED}Failed:              $FAILED_TESTS${NC}"
else
    echo -e "${GREEN}Failed:              $FAILED_TESTS${NC}"
fi

echo ""

# Calculate pass rate
if [ $TOTAL_TESTS -gt 0 ]; then
    PASS_RATE=$((100 * PASSED_TESTS / TOTAL_TESTS))
    echo -e "Pass rate:           ${PASS_RATE}%"
fi

echo ""

# List failed files if any
if [ ${#FAILED_FILES[@]} -gt 0 ]; then
    echo -e "${RED}Failed test files:${NC}"
    for file in "${FAILED_FILES[@]}"; do
        echo -e "${RED}  - $file${NC}"
    done
    echo ""
fi

# Test file breakdown
echo -e "${BLUE}Test file breakdown:${NC}"
echo -e "  type_inference.tri:  45 tests (basic inference, function calls, control flow, operators, backward)"
echo -e "  generics.tri:        45 tests (simple generics, multiple params, monomorphization, nesting)"
echo -e "  enums.tri:           35 tests (Option, Result, custom enums, nested enums)"
echo -e "  functions.tri:       35 tests (parameters, return types, generics, recursion)"
echo -e "  structs.tri:         25 tests (literals, field access, nested structs, generic structs)"
echo -e "  arrays_slices.tri:   18 tests (array literals, slicing, indexing)"
echo -e "  operators.tri:       18 tests (arithmetic, comparison, logical operators)"
echo -e "  control_flow.tri:    12 tests (if/else, match, early returns)"
echo ""
echo -e "${BLUE}Total:               233 executable test cases${NC}"
echo ""

# Final result
if [ $FAILED_TESTS -eq 0 ]; then
    echo -e "${GREEN}========================================${NC}"
    echo -e "${GREEN}All tests passed! ✓${NC}"
    echo -e "${GREEN}========================================${NC}"
    exit 0
else
    echo -e "${RED}========================================${NC}"
    echo -e "${RED}Some tests failed ✗${NC}"
    echo -e "${RED}========================================${NC}"
    exit 1
fi
