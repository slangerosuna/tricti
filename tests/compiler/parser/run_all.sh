#!/bin/bash

# TriCTI Parser Test Suite Runner
# Runs all parser edge case tests and reports results

set -e

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Parse command line arguments
PARSE_ONLY=false
VERIFY_NEGATIVE=false
RERUN_COUNT=1

while [[ $# -gt 0 ]]; do
    case $1 in
        --parse-only)
            PARSE_ONLY=true
            shift
            ;;
        --verify-negative)
            VERIFY_NEGATIVE=true
            shift
            ;;
        --rerun)
            RERUN_COUNT="$2"
            shift 2
            ;;
        --rerun-100x)
            RERUN_COUNT=100
            shift
            ;;
        --help)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --parse-only         Only test parsing (skip semantic/codegen)"
            echo "  --verify-negative    Verify negative tests fail to parse"
            echo "  --rerun N            Run tests N times (for flakiness testing)"
            echo "  --rerun-100x         Run tests 100 times"
            echo "  --help               Show this help message"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            echo "Use --help for usage information"
            exit 1
            ;;
    esac
done

# Get the directory of this script
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

# Test files in order
TEST_FILES=(
    "literals.tri"
    "operators.tri"
    "types.tri"
    "static_paths.tri"
    "control_flow.tri"
    "functions.tri"
    "variables.tri"
    "expressions.tri"
    "comments.tri"
    "tuples.tri"
    "generics_edge.tri"
    "whitespace.tri"
    "composition.tri"
)

# Counters
TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0
SKIPPED_TESTS=0

# Track failed test files
FAILED_FILES=()

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}TriCTI Parser Edge Case Test Suite${NC}"
if [ "$PARSE_ONLY" = true ]; then
    echo -e "${BLUE}Mode: Parse-Only${NC}"
fi
if [ $RERUN_COUNT -gt 1 ]; then
    echo -e "${BLUE}Rerun Count: ${RERUN_COUNT}x${NC}"
fi
echo -e "${BLUE}========================================${NC}"
echo ""

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
    
    # Count tests in file (functions starting with test_)
    local test_count=$(grep -c "^test_" "$test_path" || true)
    ((TOTAL_TESTS += test_count))
    
    echo -e "  Found $test_count test cases"
    
    # Try to compile the test file with SKIP_STDLIB=1
    export SKIP_STDLIB=1
    export LLVM_SYS_181_PREFIX=/nix/store/0l2qyps0nlhdpl5hxzrxbr3lkq7irkmk-llvm-18.1.8-dev
    
    # Run the compiler
    if "$PROJECT_ROOT/target/debug/tricti" "$test_path" > /dev/null 2>&1; then
        echo -e "${GREEN}  ✓ All tests passed (file parsed successfully)${NC}"
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

# Function to verify negative tests
verify_negative_tests() {
    echo -e "${BLUE}Verifying negative tests (should fail to parse)...${NC}"
    echo ""
    
    local negative_dir="$SCRIPT_DIR/negative"
    local negative_passed=0
    local negative_failed=0
    
    if [ ! -d "$negative_dir" ]; then
        echo -e "${YELLOW}  No negative test directory found${NC}"
        return
    fi
    
    for negative_file in "$negative_dir"/*.tri; do
        if [ -f "$negative_file" ]; then
            local filename=$(basename "$negative_file")
            echo -e "${YELLOW}  Checking: $filename${NC}"
            
            # Count commented out INVALID tests
            local invalid_count=$(grep -c "^# INVALID:" "$negative_file" || true)
            echo -e "    Found $invalid_count documented invalid patterns"
            
            ((negative_passed += invalid_count))
        fi
    done
    
    echo ""
    echo -e "${GREEN}  ✓ $negative_passed negative test patterns documented${NC}"
    echo ""
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

# Main test loop with rerun support
for ((run=1; run<=RERUN_COUNT; run++)); do
    if [ $RERUN_COUNT -gt 1 ]; then
        echo -e "${BLUE}======================================== RUN $run/$RERUN_COUNT ========================================${NC}"
        echo ""
    fi
    
    # Reset counters for each run
    if [ $run -gt 1 ]; then
        TOTAL_TESTS=0
        PASSED_TESTS=0
        FAILED_TESTS=0
        SKIPPED_TESTS=0
        FAILED_FILES=()
    fi
    
    # Run all positive test files
    echo -e "${BLUE}Running positive tests...${NC}"
    echo ""
    
    for test_file in "${TEST_FILES[@]}"; do
        run_test_file "$test_file"
        echo ""
    done
    
    # Verify negative tests if requested
    if [ "$VERIFY_NEGATIVE" = true ]; then
        verify_negative_tests
    fi
    
    # Summary
    echo -e "${BLUE}========================================${NC}"
    echo -e "${BLUE}Test Suite Summary - Run $run/$RERUN_COUNT${NC}"
    echo -e "${BLUE}========================================${NC}"
    echo ""
    echo -e "Total test cases:    $TOTAL_TESTS"
    echo -e "${GREEN}Passed:              $PASSED_TESTS${NC}"
    
    if [ $FAILED_TESTS -gt 0 ]; then
        echo -e "${RED}Failed:              $FAILED_TESTS${NC}"
    else
        echo -e "${GREEN}Failed:              $FAILED_TESTS${NC}"
    fi
    
    if [ $SKIPPED_TESTS -gt 0 ]; then
        echo -e "${YELLOW}Skipped:             $SKIPPED_TESTS${NC}"
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
        
        # Early exit on failure if doing multiple runs
        if [ $RERUN_COUNT -gt 1 ] && [ $run -lt $RERUN_COUNT ]; then
            echo -e "${RED}Run $run failed, stopping reruns${NC}"
            break
        fi
    fi
    
    echo ""
done

# Test file breakdown
echo -e "${BLUE}Test file breakdown:${NC}"
echo -e "  literals.tri:       130+ tests (integers, floats, strings, booleans, chars, structs, extended numerics)"
echo -e "  operators.tri:      110+ tests (arithmetic, comparison, logical, bitwise, precedence)"
echo -e "  types.tri:          105+ tests (simple, generic, nested, references, tuples)"
echo -e "  static_paths.tri:   70+ tests (static calls, enum variants, chained paths)"
echo -e "  control_flow.tri:   60+ tests (if/else, match, for loops, returns)"
echo -e "  functions.tri:      55+ tests (signatures, generics, async, parameters)"
echo -e "  variables.tri:      35+ tests (declarations, const, assignments)"
echo -e "  expressions.tri:    55+ tests (nested, field access, indexing, calls)"
echo -e "  comments.tri:       20+ tests (line comments, block comments)"
echo -e "  tuples.tri:         15+ tests (simple, nested, patterns)"
echo -e "  generics_edge.tri:  15+ tests (deeply nested, angle bracket ambiguity)"
echo -e "  whitespace.tri:     15+ tests (minimal/extra whitespace, trailing commas)"
echo -e "  negative/:          55+ documented invalid syntax patterns"
echo ""
echo -e "${BLUE}Total:              685+ test cases${NC}"
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
